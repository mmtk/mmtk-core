//! This module provides the trait [`Slot`] and related traits and types which allow VMs to
//! customize the layout of slots and the behavior of loading and updating object references in
//! slots.

use std::hash::Hash;
use std::marker::PhantomData;
use std::{fmt::Debug, ops::Range};

use atomic::Atomic;

use crate::util::constants::{BYTES_IN_ADDRESS, LOG_BYTES_IN_ADDRESS};
use crate::util::{Address, ObjectReference};

/// `Slot` is an abstraction for MMTk to load and update object references in memory.
///
/// # Slots and the `Slot` trait
///
/// In a VM, a slot can contain an object reference or a non-reference value.  It can be in an
/// object (a.k.a. a field), on the stack (i.e. a local variable) or in any other places (such as
/// global variables).  It may have different representations in different VMs.  Some VMs put a
/// direct pointer to an object into a slot, while others may use compressed pointers, tagged
/// pointers, offsetted pointers, etc.  Some VMs (such as JVM) have null references, and others
/// (such as CRuby and JavaScript engines) can also use tagged bits to represent non-reference
/// values such as small integers, `true`, `false`, `null` (a.k.a. "none", "nil", etc.),
/// `undefined`, etc.
///
/// In MMTk, the `Slot` trait is intended to abstract out such different representations of
/// reference fields (compressed, tagged, offsetted, etc.) among different VMs.  From MMTk's point
/// of view, **MMTk only cares about the object reference held inside the slot, but not
/// non-reference values**, such as `null`, `true`, etc.  When the slot is holding an object
/// reference, we can load the object reference from it, and we can update the object reference in
/// it after the GC moves the object.
///
/// # The `Slot` trait has pointer semantics
///
/// A `Slot` value *points to* a slot, and is not the slot itself.  In fact, the simplest
/// implementation of the `Slot` trait ([`SimpleSlot`], see below) can simply contain the address of
/// the slot.
///
/// A `Slot` can be [copied](std::marker::Copy), and the copied `Slot` instance points to the same
/// slot.
///
/// # How to implement `Slot`?
///
/// If a reference field of a VM is just a word that holds the pointer to an object, and uses the 0
/// word as the null pointer, it can use the default [`SimpleSlot`] we provide.  It simply contains
/// a pointer to a memory location that holds an address.
///
/// ```rust
/// pub struct SimpleSlot {
///     slot_addr: *mut Atomic<Address>,
/// }
/// ```
///
/// In other cases, the VM need to implement its own `Slot` instances.
///
/// For example:
/// -   The VM uses **compressed pointers** (Compressed OOPs in OpenJDK's terminology), where the
///     heap size is limited, and a 64-bit pointer is stored in a 32-bit slot.
/// -   The VM uses **tagged pointers**, where some bits of a word are used as metadata while the
///     rest are used as pointer.
/// -   The VM uses **offsetted pointers**, i.e. the value of the field is an address at an offset
///     from the [`ObjectReference`] of the target object.  Such offsetted pointers are usually used
///     to represent **interior pointers**, i.e. pointers to an object field, an array element, etc.
///
/// If needed, the implementation of `Slot` can contain not only the pointer, but also additional
/// information. The `OffsetSlot` example below also contains an offset which can be used when
/// decoding the pointer. See `src/vm/tests/mock_tests/mock_test_slots.rs` for more concrete
/// examples.
///
/// ```rust
/// pub struct OffsetSlot {
///     slot_addr: *mut Atomic<Address>,
///     offset: usize,
/// }
/// ```
///
/// When loading, `Slot::load` shall load the value from the slot and decode the value into a
/// regular `ObjectReference` (note that MMTk has specific requirements for `ObjectReference`, such
/// as being aligned, pointing inside an object, and cannot be null.  Please read the doc comments
/// of [`ObjectReference`] for details).  The decoding is VM-specific, but usually involves removing
/// tag bits and/or adding an offset to the word, and (in the case of compressed pointers) extending
/// the word size.  By doing this conversion, MMTk can implement GC algorithms in a VM-neutral way,
/// knowing only `ObjectReference`.
///
/// When GC moves object, `Slot::store` shall convert the updated `ObjectReference` back to the
/// slot-specific representation.  Compressed pointers remain compressed; tagged pointers preserve
/// their tag bits; and offsetted pointers keep their offsets.
///
/// # Performance notes
///
/// The methods of this trait are called on hot paths.  Please ensure they have high performance.
///
/// # About weak references
///
/// This trait only concerns the representation (i.e. the shape) of the slot, not its semantics,
/// such as whether it holds strong or weak references.  Therefore, one `Slot` implementation can be
/// used for both slots that hold strong references and slots that hold weak references.
pub trait Slot: Copy + Send + Debug + PartialEq + Eq + Hash {
    /// Load object reference from the slot.
    ///
    /// If the slot is not holding an object reference (For example, if it is holding NULL or a
    /// tagged non-reference value.  See trait-level doc comment.), this method should return
    /// `None`.
    ///
    /// If the slot holds an object reference with tag bits, the returned value shall be the object
    /// reference with the tag bits removed.
    fn load(&self) -> Option<ObjectReference>;

    /// Store the object reference `object` into the slot.
    ///
    /// If the slot holds an object reference with tag bits, this method must preserve the tag
    /// bits while updating the object reference so that it points to the forwarded object given by
    /// the parameter `object`.
    ///
    /// FIXME: This design is inefficient for handling object references with tag bits.  Consider
    /// introducing a new updating function to do the load, trace and store in one function.
    /// See: <https://github.com/mmtk/mmtk-core/issues/1033>
    ///
    /// FIXME: This method is currently used by both moving GC algorithms and the subsuming write
    /// barrier ([`crate::memory_manager::object_reference_write`]).  The two reference writing
    /// operations have different semantics, and need to be implemented differently if the VM
    /// supports offsetted or tagged references.
    /// See: <https://github.com/mmtk/mmtk-core/issues/1038>
    fn store(&self, object: ObjectReference);

    /// Prefetch the slot so that a subsequent `load` will be faster.
    fn prefetch_load(&self) {
        // no-op by default
    }

    /// Prefetch the slot so that a subsequent `store` will be faster.
    fn prefetch_store(&self) {
        // no-op by default
    }
}

/// A simple slot implementation that represents a word-sized slot which holds the raw address of
/// an `ObjectReference`, or 0 if it is holding a null reference.
///
/// It is the default slot type, and should be suitable for most VMs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SimpleSlot {
    slot_addr: *mut Atomic<Address>,
}

impl SimpleSlot {
    /// Create a simple slot from an address.
    ///
    /// Arguments:
    /// *   `address`: The address in memory where an `ObjectReference` is stored.
    pub fn from_address(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
        }
    }

    /// Get the address of the slot.
    ///
    /// Return the address at which the `ObjectReference` is stored.
    pub fn as_address(&self) -> Address {
        Address::from_mut_ptr(self.slot_addr)
    }
}

unsafe impl Send for SimpleSlot {}

impl Slot for SimpleSlot {
    fn load(&self) -> Option<ObjectReference> {
        let addr = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        ObjectReference::from_raw_address(addr)
    }

    fn store(&self, object: ObjectReference) {
        unsafe { (*self.slot_addr).store(object.to_raw_address(), atomic::Ordering::Relaxed) }
    }
}

/// For backword compatibility, we let `Address` implement `Slot` with the same semantics as
/// [`SimpleSlot`] so that existing bindings that use `Address` as `Slot` can continue to work.
///
/// However, we should use `SimpleSlot` directly instead of using `Address`.  The purpose of the
/// `Address` type is to represent an address in memory.  It is not directly related to fields
/// that hold references to other objects.  Calling `load()` and `store()` on an `Address` does
/// not indicate how many bytes to load or store, or how to interpret those bytes.  On the other
/// hand, `SimpleSlot` is all about how to access a field that holds a reference represented
/// simply as an `ObjectReference`.  The intention and the semantics are clearer with
/// `SimpleSlot`.
impl Slot for Address {
    fn load(&self) -> Option<ObjectReference> {
        let addr = unsafe { Address::load(*self) };
        ObjectReference::from_raw_address(addr)
    }

    fn store(&self, object: ObjectReference) {
        unsafe { Address::store(*self, object) }
    }
}

#[test]
fn a_simple_slot_should_have_the_same_size_as_a_pointer() {
    assert_eq!(
        std::mem::size_of::<SimpleSlot>(),
        std::mem::size_of::<*mut libc::c_void>()
    );
}

/// A abstract memory slice represents a piece of **heap** memory which may contains many slots.
pub trait MemorySlice: Send + Debug + PartialEq + Eq + Clone + Hash {
    /// The associate type to define how to access slots from a memory slice.
    type SlotType: Slot;
    /// The associate type to define how to iterate slots in a memory slice.
    type SlotIterator: Iterator<Item = Self::SlotType>;
    /// Iterate object slots within the slice. If there are non-reference values in the slice, the iterator should skip them.
    fn iter_slots(&self) -> Self::SlotIterator;
    /// The object which this slice belongs to. If we know the object for the slice, we will check the object state (e.g. mature or not), rather than the slice address.
    /// Normally checking the object and checking the slice does not make a difference, as the slice is part of the object (in terms of memory range). However,
    /// if a slice is in a different location from the object, the object state and the slice can be hugely different, and providing a proper implementation
    /// of this method for the owner object is important.
    fn object(&self) -> Option<ObjectReference>;
    /// Start address of the memory slice
    fn start(&self) -> Address;
    /// Size of the memory slice
    fn bytes(&self) -> usize;
    /// Memory copy support
    fn copy(src: &Self, tgt: &Self);
}

/// Iterate slots within `Range<Address>`.
pub struct AddressRangeIterator {
    cursor: Address,
    limit: Address,
}

impl Iterator for AddressRangeIterator {
    type Item = Address;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.limit {
            None
        } else {
            let slot = self.cursor;
            self.cursor += BYTES_IN_ADDRESS;
            Some(slot)
        }
    }
}

impl MemorySlice for Range<Address> {
    type SlotType = Address;
    type SlotIterator = AddressRangeIterator;

    fn iter_slots(&self) -> Self::SlotIterator {
        AddressRangeIterator {
            cursor: self.start,
            limit: self.end,
        }
    }

    fn object(&self) -> Option<ObjectReference> {
        None
    }

    fn start(&self) -> Address {
        self.start
    }

    fn bytes(&self) -> usize {
        self.end - self.start
    }

    fn copy(src: &Self, tgt: &Self) {
        debug_assert_eq!(src.bytes(), tgt.bytes());
        debug_assert_eq!(
            src.bytes() & ((1 << LOG_BYTES_IN_ADDRESS) - 1),
            0,
            "bytes are not a multiple of words"
        );
        // Raw memory copy
        unsafe {
            let words = tgt.bytes() >> LOG_BYTES_IN_ADDRESS;
            let src = src.start().to_ptr::<usize>();
            let tgt = tgt.start().to_mut_ptr::<usize>();
            std::ptr::copy(src, tgt, words)
        }
    }
}

/// Memory slice type with empty implementations.
/// For VMs that do not use the memory slice type.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct UnimplementedMemorySlice<SL: Slot = SimpleSlot>(PhantomData<SL>);

/// Slot iterator for `UnimplementedMemorySlice`.
pub struct UnimplementedMemorySliceSlotIterator<SL: Slot>(PhantomData<SL>);

impl<SL: Slot> Iterator for UnimplementedMemorySliceSlotIterator<SL> {
    type Item = SL;

    fn next(&mut self) -> Option<Self::Item> {
        unimplemented!()
    }
}

impl<SL: Slot> MemorySlice for UnimplementedMemorySlice<SL> {
    type SlotType = SL;
    type SlotIterator = UnimplementedMemorySliceSlotIterator<SL>;

    fn iter_slots(&self) -> Self::SlotIterator {
        unimplemented!()
    }

    fn object(&self) -> Option<ObjectReference> {
        unimplemented!()
    }

    fn start(&self) -> Address {
        unimplemented!()
    }

    fn bytes(&self) -> usize {
        unimplemented!()
    }

    fn copy(_src: &Self, _tgt: &Self) {
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_range_iteration() {
        let src: Vec<usize> = (0..32).collect();
        let src_slice = Address::from_ptr(&src[0])..Address::from_ptr(&src[0]) + src.len();
        for (i, v) in src_slice.iter_slots().enumerate() {
            assert_eq!(i, unsafe { v.load::<usize>() })
        }
    }

    #[test]
    fn memory_copy_on_address_ranges() {
        let src = [1u8; 32];
        let mut dst = [0u8; 32];
        let src_slice = Address::from_ptr(&src[0])..Address::from_ptr(&src[0]) + src.len();
        let dst_slice =
            Address::from_mut_ptr(&mut dst[0])..Address::from_mut_ptr(&mut dst[0]) + src.len();
        MemorySlice::copy(&src_slice, &dst_slice);
        assert_eq!(dst.iter().sum::<u8>(), src.len() as u8);
    }
}
