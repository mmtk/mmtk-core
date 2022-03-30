use std::fmt::Debug;
use std::hash::Hash;

use atomic::Atomic;

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
/// `ObjectReference` which is applicable to `ObjectModel::object_start_ref`.  The implementation
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
    fn load(&self) -> ObjectReference;

    /// Store the object reference `object` into the edge.
    fn store(&self, object: ObjectReference);

    /// Prefetch the edge so that a subsequent `load` will be faster.
    #[inline(always)]
    fn prefetch_load(&self) {
        // no-op by default
    }

    /// Prefetch the edge so that a subsequent `store` will be faster.
    #[inline(always)]
    fn prefetch_store(&self) {
        // no-op by default
    }
}

/// A simple edge implementation that represents a word-sized slot where an ObjectReference value
/// is stored as is.  It is the default edge type, and should be suitable for most VMs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SimpleEdge {
    slot_addr: *mut Atomic<ObjectReference>,
}

impl SimpleEdge {
    /// Create a simple edge from an address.
    ///
    /// Arguments:
    /// *   `address`: The address in memory where an `ObjectReference` is stored.
    #[inline(always)]
    pub fn from_address(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
        }
    }

    /// Get the address of the edge.
    ///
    /// Return the address at which the `ObjectReference` is stored.
    #[inline(always)]
    pub fn as_address(&self) -> Address {
        Address::from_mut_ptr(self.slot_addr)
    }
}

unsafe impl Send for SimpleEdge {}

impl Edge for SimpleEdge {
    #[inline(always)]
    fn load(&self) -> ObjectReference {
        unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) }
    }

    #[inline(always)]
    fn store(&self, object: ObjectReference) {
        unsafe { (*self.slot_addr).store(object, atomic::Ordering::Relaxed) }
    }
}

/// For backword compatibility.
/// We let Address implement Edge so existing bindings that use `Address` to represent an edge can
/// continue to work.
impl Edge for Address {
    #[inline(always)]
    fn load(&self) -> ObjectReference {
        unsafe { Address::load(*self) }
    }

    #[inline(always)]
    fn store(&self, object: ObjectReference) {
        unsafe { Address::store(*self, object) }
    }
}
