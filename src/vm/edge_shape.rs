use std::hash::Hash;
use std::marker::PhantomData;
use std::{fmt::Debug, ops::Range};

use atomic::Atomic;

use crate::util::constants::{BYTES_IN_ADDRESS, LOG_BYTES_IN_ADDRESS};
use crate::util::{Address, ObjectReference};

/// An `Edge` represents a slot in an object (a.k.a. a field), on the stack (i.e. a local variable)
/// or any other places (such as global variables).  A slot may hold an object reference. We can
/// load the object reference from it, and we can store an ObjectReference into it.  For some VMs,
/// a slot may sometimes not hold an object reference.  For example, it can hold a special `NULL`
/// pointer which does not point to any object, or it can hold a tagged non-reference value, such
/// as small integers and special values such as `true`, `false`, `null` (a.k.a. "none", "nil",
/// etc. for other VMs), `undefined`, etc.
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
    /// Load object reference from the slot.
    ///
    /// If the slot is not holding an object reference (For example, if it is holding NULL or a
    /// tagged non-reference value.  See trait-level doc comment.), this method should return
    /// `ObjectReference::NULL`.
    ///
    /// If the slot holds an object reference with tag bits, the returned value shall be the object
    /// reference with the tag bits removed.
    fn load(&self) -> ObjectReference;

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
