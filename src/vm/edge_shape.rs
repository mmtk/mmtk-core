use std::hash::Hash;
use std::marker::PhantomData;
use std::{fmt::Debug, ops::Range};

use atomic::Atomic;

use crate::util::constants::{BYTES_IN_ADDRESS, LOG_BYTES_IN_ADDRESS};
use crate::util::{Address, ObjectReference};

/// An abstract edge.  An edge holds an object reference.  When we load from it, we get an
/// ObjectReference; we can also store an ObjectReference into it.
///
/// This intends to abstract out the differences of reference field representation among different
/// VMs.  If the VM represent a reference field as a word that holds the pointer to the object, it
/// can use the default `SimpleEdge` we provide.  In some cases, the VM need to implement its own
/// `Edge` instances.
///
/// For example:
/// -   The VM uses compressed pointer (Compressed OOP in OpenJDK's terminology), where the heap
///     size is limited, and a 64-bit pointer is stored in a 32-bit slot.
/// -   The VM uses tagged pointer, where some bits of a word are used as metadata while the rest
///     are used as pointer.
/// -   A field holds a pointer to the middle of an object (an object field, or an array element,
///     or some arbitrary offset) for some reasons.
///
/// When loading, `Edge::load` shall decode its internal representation to a "regular"
/// `ObjectReference`.  The implementation
/// can do this with any appropriate operations, usually shifting and masking bits or subtracting
/// offset from the address.  By doing this conversion, MMTk can implement GC algorithms in a
/// VM-neutral way, knowing only `ObjectReference`.
///
/// When GC moves object, `Edge::store` shall convert the updated `ObjectReference` back to the
/// edge-specific representation.  Compressed pointers remain compressed; tagged pointers preserve
/// their tag bits; and offsetted pointers keep their offsets.
///
/// The methods of this trait are called on hot paths.  Please ensure they have high performance.
/// Use inlining when appropriate.
///
/// Note: this trait only concerns the representation (i.e. the shape) of the edge, not its
/// semantics, such as whether it holds strong or weak references.  If a VM holds a weak reference
/// in a word as a pointer, it can also use `SimpleEdge` for weak reference fields.
pub trait Edge: Copy + Send + Debug + PartialEq + Eq + Hash {
    /// Load object reference from the edge.
    ///
    /// If the slot is not holding an object reference, it should return `ObjectReference::NULL`.
    /// Specifically,
    ///
    /// -   If the langauge has the concept of "null pointer" which does not point to any object in
    ///     the heap, this method should return `ObjectReference::NULL` regardless how a null
    ///     pointer is encoded in the VM.  However, if the VM uses a special object in the heap to
    ///     represent a null value, such as the `None` object of `NoneType` in Python, this method
    ///     should still return the object reference to such `None` objects so that they are
    ///     properly traced, kept alive, and they have their references forwarded.
    /// -   If, in a VM, the data type a slot can hold is a union of references and non-reference
    ///     values, and the slot is currently holding a non-reference value, such as a small
    ///     integer, a floating-point number, or any special value such as `true`, `false` or `nil`
    ///     that do not point to any object, the slot is considered not holding an reference.  This
    ///     method should return `ObjectReference::NULL` in such cases.
    ///
    /// If the slot holds an object reference with tag bits, the returned value shall be the object
    /// reference with the tag bits removed.
    fn load(&self) -> ObjectReference;

    /// Store the object reference `object` into the edge.
    ///
    /// FIXME: Currently the subsuming write barrier (`Barrier::object_reference_write`) calls this
    /// method to perform the actual store to the field.  It only works if the VM does not store
    /// tag bits in the slot.
    ///
    /// FIXME: If the slot contains tag bits, consider overriding the `update_for_forwarding`
    /// method. See: https://github.com/mmtk/mmtk-core/issues/1033
    fn store(&self, object: ObjectReference);

    /// Update the slot for forwarding.
    ///
    /// If the slot is holding an object reference, this method shall call `updater` with that
    /// reference; if the slot is not holding an object reference, including when holding a NULL
    /// pointer or holding small integers or special non-reference values such as `true`, `false`
    /// or `nil`, this method does not need to take further action.  In no circumstance should this
    /// method pass `ObjectReference::NULL` to `updater`.
    ///
    /// If the returned value of the `updater` closure is not `ObjectReference::NULL`, it will be
    /// the forwarded object reference of the original object, and this method shall update the
    /// slot to point to the new location; if the returned value is `ObjectReference::NULL`, this
    /// method does not need to take further action.
    ///
    /// This method is called to trace the object pointed by this slot, and forward the reference
    /// in the slot.  To implement this semantics, the VM should usually preserve the tag bits if
    /// it uses tagged pointers to indicate whether the slot holds an object reference or
    /// non-reference values.
    ///
    /// The default implementation calls `self.load()` to load the object reference, and calls
    /// `self.store` to store the updated reference.  VMs that use tagged pointers should override
    /// this method to preserve the tag bits between the load and store.
    fn update_for_forwarding<F>(&self, updater: F)
    where
        F: FnOnce(ObjectReference) -> ObjectReference,
    {
        let object = self.load();
        if object.is_null() {
            return;
        }
        let new_object = updater(object);
        if new_object.is_null() {
            return;
        }
        self.store(new_object);
    }

    /// Prefetch the edge so that a subsequent `load` will be faster.
    fn prefetch_load(&self) {
        // no-op by default
    }

    /// Prefetch the edge so that a subsequent `store` will be faster.
    fn prefetch_store(&self) {
        // no-op by default
    }
}

/// A simple edge implementation that represents a word-sized slot where an ObjectReference value
/// is stored as is.  It is the default edge type, and should be suitable for most VMs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SimpleEdge {
    slot_addr: *mut Atomic<ObjectReference>,
}

impl SimpleEdge {
    /// Create a simple edge from an address.
    ///
    /// Arguments:
    /// *   `address`: The address in memory where an `ObjectReference` is stored.
    pub fn from_address(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
        }
    }

    /// Get the address of the edge.
    ///
    /// Return the address at which the `ObjectReference` is stored.
    pub fn as_address(&self) -> Address {
        Address::from_mut_ptr(self.slot_addr)
    }
}

unsafe impl Send for SimpleEdge {}

impl Edge for SimpleEdge {
    fn load(&self) -> ObjectReference {
        unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) }
    }

    fn store(&self, object: ObjectReference) {
        unsafe { (*self.slot_addr).store(object, atomic::Ordering::Relaxed) }
    }
}

/// For backword compatibility, we let `Address` implement `Edge` so that existing bindings that
/// use `Address` to represent an edge can continue to work.
///
/// However, we should use `SimpleEdge` directly instead of using `Address`.  The purpose of the
/// `Address` type is to represent an address in memory.  It is not directly related to fields
/// that hold references to other objects.  Calling `load()` and `store()` on an `Address` does
/// not indicate how many bytes to load or store, or how to interpret those bytes.  On the other
/// hand, `SimpleEdge` is all about how to access a field that holds a reference represented
/// simply as an `ObjectReference`.  The intention and the semantics are clearer with
/// `SimpleEdge`.
impl Edge for Address {
    fn load(&self) -> ObjectReference {
        unsafe { Address::load(*self) }
    }

    fn store(&self, object: ObjectReference) {
        unsafe { Address::store(*self, object) }
    }
}

#[test]
fn a_simple_edge_should_have_the_same_size_as_a_pointer() {
    assert_eq!(
        std::mem::size_of::<SimpleEdge>(),
        std::mem::size_of::<*mut libc::c_void>()
    );
}

/// A abstract memory slice represents a piece of **heap** memory.
pub trait MemorySlice: Send + Debug + PartialEq + Eq + Clone + Hash {
    /// The associate type to define how to access edges from a memory slice.
    type Edge: Edge;
    /// The associate type to define how to iterate edges in a memory slice.
    type EdgeIterator: Iterator<Item = Self::Edge>;
    /// Iterate object edges within the slice. If there are non-reference values in the slice, the iterator should skip them.
    fn iter_edges(&self) -> Self::EdgeIterator;
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

/// Iterate edges within `Range<Address>`.
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
            let edge = self.cursor;
            self.cursor += BYTES_IN_ADDRESS;
            Some(edge)
        }
    }
}

impl MemorySlice for Range<Address> {
    type Edge = Address;
    type EdgeIterator = AddressRangeIterator;

    fn iter_edges(&self) -> Self::EdgeIterator {
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
pub struct UnimplementedMemorySlice<E: Edge = SimpleEdge>(PhantomData<E>);

/// Edge iterator for `UnimplementedMemorySlice`.
pub struct UnimplementedMemorySliceEdgeIterator<E: Edge>(PhantomData<E>);

impl<E: Edge> Iterator for UnimplementedMemorySliceEdgeIterator<E> {
    type Item = E;

    fn next(&mut self) -> Option<Self::Item> {
        unimplemented!()
    }
}

impl<E: Edge> MemorySlice for UnimplementedMemorySlice<E> {
    type Edge = E;
    type EdgeIterator = UnimplementedMemorySliceEdgeIterator<E>;

    fn iter_edges(&self) -> Self::EdgeIterator {
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
        for (i, v) in src_slice.iter_edges().enumerate() {
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
