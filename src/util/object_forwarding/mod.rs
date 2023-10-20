use crate::util::copy::*;
use crate::util::ObjectReference;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use std::marker::PhantomData;

mod traditional;

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
    #[cfg(feature = "vm_forwarding")]
    vm_data: <VM::VMObjectModel as ObjectModel<VM>>::VMForwardingDataType,
    phantom_data: PhantomData<VM>,
}

/// Provide states and methods for a lost forwarding attempt.
///
/// See [`ForwardingAttempt`]
#[must_use]
pub struct LostForwardingAttempt<VM: VMBinding> {
    /// The object to forward.  This field holds the from-space address.
    object: ObjectReference,
    #[cfg(not(feature = "vm_forwarding"))]
    old_state: u8,
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> ForwardingAttempt<VM> {
    /// Attempt to forward an object by atomically transitioning the forwarding state of `object`
    /// from `FORWARDING_NOT_TRIGGERED_YET` to `BEING_FORWARDED`.
    pub fn attempt(object: ObjectReference) -> Self {
        #[cfg(not(feature = "vm_forwarding"))]
        {
            let old_state = traditional::attempt_to_forward::<VM>(object);
            if traditional::state_is_forwarded_or_being_forwarded(old_state) {
                Self::Lost(LostForwardingAttempt {
                    object,
                    old_state,
                    phantom_data: PhantomData,
                })
            } else {
                Self::Won(WonForwardingAttempt {
                    object,
                    phantom_data: PhantomData,
                })
            }
        }

        #[cfg(feature = "vm_forwarding")]
        {
            match VM::VMObjectModel::attempt_to_forward(object) {
                Some(vm_data) => Self::Won(WonForwardingAttempt {
                    object,
                    vm_data,
                    phantom_data: PhantomData,
                }),
                None => Self::Lost(LostForwardingAttempt {
                    object,
                    phantom_data: PhantomData,
                }),
            }
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
        #[cfg(not(feature = "vm_forwarding"))]
        {
            let new_object = VM::VMObjectModel::copy(self.object, semantics, copy_context);
            traditional::write_forwarding_bits_and_forwarding_pointer::<VM>(self.object, new_object);
            new_object
        }

        #[cfg(feature = "vm_forwarding")]
        {
            let new_object =
                VM::VMObjectModel::copy(self.object, semantics, copy_context, self.vm_data);
            VM::VMObjectModel::write_forwarding_state_and_forwarding_pointer(
                self.object,
                new_object,
            );
            new_object
        }
    }

    /// Call this to revert the forwarding state to the state before calling `attempt`
    pub fn revert(self) {
        #[cfg(not(feature = "vm_forwarding"))]
        traditional::clear_forwarding_bits::<VM>(self.object);

        #[cfg(feature = "vm_forwarding")]
        VM::VMObjectModel::revert_forwarding_state(self.object, self.vm_data);
    }
}

impl<VM: VMBinding> LostForwardingAttempt<VM> {
    /// Spin-wait for the object's forwarding to become complete and then read the forwarding
    /// pointer to the new object.  If the forwarding state is reverted, this function will simply
    /// return `self.object`.
    pub fn spin_and_get_forwarded_object(self) -> ObjectReference {
        // About the curly braces: The Rust compiler seems not to know that
        // `not(feature = "vm_forwarding")` and `feature = "vm_forwarding"` are mutually exclusive.
        // It will suggest adding semicolon (which is wrong) if we remove the curly braces.
        #[cfg(not(feature = "vm_forwarding"))]
        {
            traditional::spin_and_get_forwarded_object::<VM>(self.object, self.old_state)
        }

        #[cfg(feature = "vm_forwarding")]
        {
            VM::VMObjectModel::spin_and_get_forwarded_object(self.object)
        }
    }
}

pub fn is_forwarded<VM: VMBinding>(object: ObjectReference) -> bool {
    #[cfg(not(feature = "vm_forwarding"))]
    {
        traditional::is_forwarded::<VM>(object)
    }

    #[cfg(feature = "vm_forwarding")]
    {
        VM::VMObjectModel::is_forwarded(object)
    }
}


/// Read the forwarding pointer of an object.
/// This function is called on forwarded/being_forwarded objects.
pub fn read_forwarding_pointer<VM: VMBinding>(object: ObjectReference) -> ObjectReference {
    #[cfg(not(feature = "vm_forwarding"))]
    {
        traditional::read_forwarding_pointer::<VM>(object)
    }

    #[cfg(feature = "vm_forwarding")]
    {
        VM::VMObjectModel::read_forwarding_pointer(object)
    }
}

/// Clear the forwarding bits.
/// Not needed when using VM-side forwarding implementation.
#[cfg(not(feature = "vm_forwarding"))]
pub fn clear_forwarding_bits<VM: VMBinding>(object: ObjectReference) {
    traditional::clear_forwarding_bits::<VM>(object);
}
