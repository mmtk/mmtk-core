use crate::util::copy::*;
use crate::util::metadata::MetadataSpec;
use crate::util::{constants, ObjectReference};
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::atomic::Ordering;

const FORWARDING_NOT_TRIGGERED_YET: u8 = 0b00;
const BEING_FORWARDED: u8 = 0b10;
const FORWARDED: u8 = 0b11;
const FORWARDING_MASK: u8 = 0b11;
#[allow(unused)]
const FORWARDING_BITS: usize = 2;

// copy address mask
#[cfg(target_pointer_width = "64")]
const FORWARDING_POINTER_MASK: usize = 0x00ff_ffff_ffff_fff8;
#[cfg(target_pointer_width = "32")]
const FORWARDING_POINTER_MASK: usize = 0xffff_fffc;

/// This type, together with `WonForwardingAttempt` and `LostForwardingAttempt`, represents an
/// attempt to forward an object, and provides methods for manipulating the forwarding state and
/// forwarding pointer of an object in an idiomatic way.
///
/// A GC worker thread initiates object forwarding by calling `ForwardingAttempt::attempt(object)`.
/// It will try to atomically transition the forwarding state from `FORWARDING_NOT_TRIGGERED_YET`
/// to `BEING_FORWARDED`.
///
/// -   If the transition is successful (i.e. we "won" the race), the GC worker gains exclusive
///     right to further transition its forwarding state.
///
///     -   The GC worker can finish the forwarding by calling
///         `WonForwardingAttempt::forward_object`.  It will actually copy the object and set the
///         forwarding bits and forwarding pointers.
///
///     -   The GC worker can abort the forwarding by calling `WonForwardingAttempt::revert`.  It
///         will revert the forwarding state to the state before calling
///         `ForwardingAttempt::attempt`.
///
/// -   If the transition failed (i.e. we "lost" the race), it means another GC worker is
///     forwarding or has forwarded the object.
///
///     -   The current GC worker can call `LostForwardingAttempt::spin_and_get_forwarded_object`
///         to wait until the other GC worker finished forwarding, and read the forwarding pointer.
#[must_use]
pub enum ForwardingAttempt<VM: VMBinding> {
    /// The old state before `Self::attempt` was `FORWARDING_NOT_TRIGGERED_YET`.
    /// It means the current thread transitioned the forwarding state from
    /// `FORWARDING_NOT_TRIGGERED_YET` to `BEING_FORWARDED`.
    Won(WonForwardingAttempt<VM>),
    /// The old state before `Self::attempt` was `BEING_FORWARDED` or `FORWARDED`.
    /// It means another thread is forwarding or has already forwarded the object.
    Lost(LostForwardingAttempt<VM>),
}

/// Provide states and methods for a won forwarding attempt.
///
/// See [`ForwardingAttempt`]
#[must_use]
pub struct WonForwardingAttempt<VM: VMBinding> {
    /// The object to forward.  This field holds the from-space address.
    object: ObjectReference,
    /// VM-specific temporary data used for reverting the forwarding states.
    vm_data: usize,
    phantom_data: PhantomData<VM>,
}

/// Provide states and methods for a lost forwarding attempt.
///
/// See [`ForwardingAttempt`]
#[must_use]
pub struct LostForwardingAttempt<VM: VMBinding> {
    /// The object to forward.  This field holds the from-space address.
    object: ObjectReference,
    old_state: u8,
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> ForwardingAttempt<VM> {
    /// Attempt to forward an object by atomically transitioning the forwarding state of `object`
    /// from `FORWARDING_NOT_TRIGGERED_YET` to `BEING_FORWARDED`.
    pub fn attempt(object: ObjectReference) -> Self {
        let old_value = attempt_to_forward::<VM>(object);

        if is_forwarded_or_being_forwarded::<VM>(object) {
            Self::Lost(LostForwardingAttempt {
                object,
                old_state: old_value,
                phantom_data: PhantomData,
            })
        } else {
            Self::Won(WonForwardingAttempt {
                object,
                vm_data: 0,
                phantom_data: PhantomData,
            })
        }
    }
}

impl<VM: VMBinding> WonForwardingAttempt<VM> {
    /// Call this to forward the object.
    ///
    /// This method will also set the forwarding state to `FORWARDED` and store the forwarding
    /// pointer at the appropriate location.
    pub fn forward_object(
        self,
        semantics: CopySemantics,
        copy_context: &mut GCWorkerCopyContext<VM>,
    ) -> ObjectReference {
        let new_object = VM::VMObjectModel::copy(self.object, semantics, copy_context);
        write_forwarding_bits_and_forwarding_pointer::<VM>(self.object, new_object);
        new_object
    }

    /// Call this to revert the forwarding state to the state before calling `attempt`
    pub fn revert(self) {
        clear_forwarding_bits::<VM>(self.object);
    }
}

impl<VM: VMBinding> LostForwardingAttempt<VM> {
    /// Spin-wait for the object's forwarding to become complete and then read the forwarding
    /// pointer to the new object.
    pub fn spin_and_get_forwarded_object(self) -> ObjectReference {
        spin_and_get_forwarded_object::<VM>(self.object, self.old_state)
    }
}

/// Attempt to become the worker thread who will forward the object.
/// The successful worker will set the object forwarding bits to BEING_FORWARDED, preventing other workers from forwarding the same object.
fn attempt_to_forward<VM: VMBinding>(object: ObjectReference) -> u8 {
    loop {
        let old_value = get_forwarding_status::<VM>(object);
        if old_value != FORWARDING_NOT_TRIGGERED_YET
            || VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    BEING_FORWARDED,
                    None,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
        {
            return old_value;
        }
    }
}

/// Spin-wait for the object's forwarding to become complete and then read the forwarding pointer to the new object.
///
/// # Arguments:
///
/// * `object`: the forwarded/being_forwarded object.
/// * `forwarding_bits`: the last state of the forwarding bits before calling this function.
///
/// Returns a reference to the new object.
///
fn spin_and_get_forwarded_object<VM: VMBinding>(
    object: ObjectReference,
    forwarding_bits: u8,
) -> ObjectReference {
    let mut forwarding_bits = forwarding_bits;
    while forwarding_bits == BEING_FORWARDED {
        forwarding_bits = get_forwarding_status::<VM>(object);
    }

    if forwarding_bits == FORWARDED {
        read_forwarding_pointer::<VM>(object)
    } else {
        // For some policies (such as Immix), we can have interleaving such that one thread clears
        // the forwarding word while another thread was stuck spinning in the above loop.
        // See: https://github.com/mmtk/mmtk-core/issues/579
        debug_assert!(
            forwarding_bits == FORWARDING_NOT_TRIGGERED_YET,
            "Invalid/Corrupted forwarding word {:x} for object {}",
            forwarding_bits,
            object,
        );
        object
    }
}

fn write_forwarding_bits_and_forwarding_pointer<VM: VMBinding>(
    object: ObjectReference,
    new_object: ObjectReference,
) {
    if let Some(shift) = forwarding_bits_offset_in_forwarding_pointer::<VM>() {
        VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.store_atomic::<VM, usize>(
            object,
            new_object.to_raw_address().as_usize() | ((FORWARDED as usize) << shift),
            None,
            Ordering::SeqCst,
        )
    } else {
        write_forwarding_pointer::<VM>(object, new_object);
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.store_atomic::<VM, u8>(
            object,
            FORWARDED,
            None,
            Ordering::SeqCst,
        );
    }
}

/// Return the forwarding bits for a given `ObjectReference`.
fn get_forwarding_status<VM: VMBinding>(object: ObjectReference) -> u8 {
    VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.load_atomic::<VM, u8>(
        object,
        None,
        Ordering::SeqCst,
    )
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    get_forwarding_status::<VM>(object) == FORWARDED
}

fn is_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    get_forwarding_status::<VM>(object) == BEING_FORWARDED
}

fn is_forwarded_or_being_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    get_forwarding_status::<VM>(object) != FORWARDING_NOT_TRIGGERED_YET
}

fn state_is_forwarded_or_being_forwarded(forwarding_bits: u8) -> bool {
    forwarding_bits != FORWARDING_NOT_TRIGGERED_YET
}

fn state_is_being_forwarded(forwarding_bits: u8) -> bool {
    forwarding_bits == BEING_FORWARDED
}

/// Zero the forwarding bits of an object.
/// This function is used on new objects.
pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.store_atomic::<VM, u8>(
        object,
        0,
        None,
        Ordering::SeqCst,
    )
}

/// Read the forwarding pointer of an object.
/// This function is called on forwarded/being_forwarded objects.
pub fn read_forwarding_pointer<VM: VMBinding>(object: ObjectReference) -> ObjectReference {
    debug_assert!(
        is_forwarded_or_being_forwarded::<VM>(object),
        "read_forwarding_pointer called for object {:?} that has not started forwarding!",
        object,
    );

    // We write the forwarding poiner. We know it is an object reference.
    unsafe {
        ObjectReference::from_raw_address(crate::util::Address::from_usize(
            VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.load_atomic::<VM, usize>(
                object,
                Some(FORWARDING_POINTER_MASK),
                Ordering::SeqCst,
            ),
        ))
    }
}

/// Write the forwarding pointer of an object.
/// This function is called on being_forwarded objects.
fn write_forwarding_pointer<VM: VMBinding>(
    object: ObjectReference,
    new_object: ObjectReference,
) {
    debug_assert!(
        is_being_forwarded::<VM>(object),
        "write_forwarding_pointer called for object {:?} that is not being forwarded! Forwarding state = 0x{:x}",
        object,
        get_forwarding_status::<VM>(object),
    );

    trace!("write_forwarding_pointer({}, {})", object, new_object);
    VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.store_atomic::<VM, usize>(
        object,
        new_object.to_raw_address().as_usize(),
        Some(FORWARDING_POINTER_MASK),
        Ordering::SeqCst,
    )
}

/// (This function is only used internal to the `util` module)
///
/// This function checks whether the forwarding pointer and forwarding bits can be written in the same atomic operation.
///
/// Returns `None` if this is not possible.
/// Otherwise, returns `Some(shift)`, where `shift` is the left shift needed on forwarding bits.
///
#[cfg(target_endian = "little")]
fn forwarding_bits_offset_in_forwarding_pointer<VM: VMBinding>() -> Option<isize> {
    use std::ops::Deref;
    // if both forwarding bits and forwarding pointer are in-header
    match (
        VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC.deref(),
        VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.deref(),
    ) {
        (MetadataSpec::InHeader(fp), MetadataSpec::InHeader(fb)) => {
            let maybe_shift = fb.bit_offset - fp.bit_offset;
            if maybe_shift >= 0 && maybe_shift < constants::BITS_IN_WORD as isize {
                Some(maybe_shift)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(target_endian = "big")]
fn forwarding_bits_offset_in_forwarding_pointer<VM: VMBinding>() -> Option<isize> {
    unimplemented!()
}
