use crate::plan::VectorObjectQueue;
use crate::scheduler::GCWorker;
#[cfg(feature = "is_mmtk_object")]
use crate::util::alloc_bit;
use crate::util::conversions;
use crate::util::*;
use crate::vm::VMBinding;
use std::marker::PhantomData;

/// Space Function Table (SFT).
///
/// This trait captures functions that reflect _space-specific per-object
/// semantics_.   These functions are implemented for each object via a special
/// space-based dynamic dispatch mechanism where the semantics are _not_
/// determined by the object's _type_, but rather, are determined by the _space_
/// that the object is in.
///
/// The underlying mechanism exploits the fact that spaces use the address space
/// at an MMTk chunk granularity with the consequence that each chunk maps to
/// exactluy one space, so knowing the chunk for an object reveals its space.
/// The dispatch then works by performing simple address arithmetic on the object
/// reference to find a chunk index which is used to index a table which returns
/// the space.   The relevant function is then dispatched against that space
/// object.
///
/// We use the SFT trait to simplify typing for Rust, so our table is a
/// table of SFT rather than Space.
pub trait SFT {
    /// The space name
    fn name(&self) -> &str;

    /// Get forwarding pointer if the object is forwarded.
    #[inline(always)]
    fn get_forwarded_object(&self, _object: ObjectReference) -> Option<ObjectReference> {
        None
    }

    /// Is the object live, determined by the policy?
    fn is_live(&self, object: ObjectReference) -> bool;

    /// Is the object reachable, determined by the policy?
    /// Note: Objects in ImmortalSpace may have `is_live = true` but are actually unreachable.
    #[inline(always)]
    fn is_reachable(&self, object: ObjectReference) -> bool {
        self.is_live(object)
    }

    // Returns true if object status unpinned => pinned
    fn pin_object(&self, object: ObjectReference) -> bool;

    // Returns true if object status pinned => unpinned
    fn unpin_object(&self, object: ObjectReference) -> bool;

    // Returns true if object status is currently pinned
    fn is_object_pinned(&self, object: ObjectReference) -> bool;

    /// Is the object movable, determined by the policy? E.g. the policy is non-moving,
    /// or the object is pinned.
    fn is_movable(&self) -> bool;

    /// Is the object sane? A policy should return false if there is any abnormality about
    /// object - the sanity checker will fail if an object is not sane.
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool;

    /// Is the object managed by MMTk? For most cases, if we find the sft for an object, that means
    /// the object is in the space and managed by MMTk. However, for some spaces, like MallocSpace,
    /// we mark the entire chunk in the SFT table as a malloc space, but only some of the addresses
    /// in the space contain actual MMTk objects. So they need a further check.
    #[inline(always)]
    fn is_in_space(&self, _object: ObjectReference) -> bool {
        true
    }

    /// Is `addr` a valid object reference to an object allocated in this space?
    /// This default implementation works for all spaces that use MMTk's mapper to allocate memory.
    /// Some spaces, like `MallocSpace`, use third-party libraries to allocate memory.
    /// Such spaces needs to override this method.
    #[cfg(feature = "is_mmtk_object")]
    #[inline(always)]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        // Having found the SFT means the `addr` is in one of our spaces.
        // Although the SFT map is allocated eagerly when the space is contiguous,
        // the pages of the space itself are acquired on demand.
        // Therefore, the page of `addr` may not have been mapped, yet.
        if !addr.is_mapped() {
            return false;
        }
        // The `addr` is mapped. We use the global alloc bit to get the exact answer.
        alloc_bit::is_alloced_object(addr)
    }

    /// Initialize object metadata (in the header, or in the side metadata).
    fn initialize_object_metadata(&self, object: ObjectReference, alloc: bool);

    /// Trace objects through SFT. This along with [`SFTProcessEdges`](mmtk/scheduler/gc_work/SFTProcessEdges)
    /// provides an easy way for most plans to trace objects without the need to implement any plan-specific
    /// code. However, tracing objects for some policies are more complicated, and they do not provide an
    /// implementation of this method. For example, mark compact space requires trace twice in each GC.
    /// Immix has defrag trace and fast trace.
    fn sft_trace_object(
        &self,
        // We use concrete type for `queue` because SFT doesn't support generic parameters,
        // and SFTProcessEdges uses `VectorObjectQueue`.
        queue: &mut VectorObjectQueue,
        object: ObjectReference,
        worker: GCWorkerMutRef,
    ) -> ObjectReference;
}

// Create erased VM refs for these types that will be used in `sft_trace_object()`.
// In this way, we can store the refs with <VM> in SFT (which cannot have parameters with generic type parameters)

use crate::util::erase_vm::define_erased_vm_mut_ref;
define_erased_vm_mut_ref!(GCWorkerMutRef = GCWorker<VM>);

/// Print debug info for SFT. Should be false when committed.
pub const DEBUG_SFT: bool = cfg!(debug_assertions) && false;

/// An empty entry for SFT.
#[derive(Debug)]
pub struct EmptySpaceSFT {}

pub const EMPTY_SFT_NAME: &str = "empty";
pub const EMPTY_SPACE_SFT: EmptySpaceSFT = EmptySpaceSFT {};

impl SFT for EmptySpaceSFT {
    fn name(&self) -> &str {
        EMPTY_SFT_NAME
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        panic!(
            "Called is_live() on {:x}, which maps to an empty space",
            object
        )
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        warn!("Object in empty space!");
        false
    }
    fn pin_object(&self, _object: ObjectReference) -> bool {
        panic!("Cannot pin/unpin objects of EmptySpace.")
    }
    fn unpin_object(&self, _object: ObjectReference) -> bool {
        panic!("Cannot pin/unpin objects of EmptySpace.")
    }
    fn is_object_pinned(&self, _object: ObjectReference) -> bool {
        false
    }
    fn is_movable(&self) -> bool {
        /*
         * FIXME steveb I think this should panic (ie the function should not
         * be invoked on an empty space).   However, JikesRVM currently does
         * call this in an unchecked way and expects 'false' for out of bounds
         * addresses.  So until that is fixed upstream, we'll return false here.
         *
         * panic!("called is_movable() on empty space")
         */
        false
    }
    #[inline(always)]
    fn is_in_space(&self, _object: ObjectReference) -> bool {
        false
    }
    #[cfg(feature = "is_mmtk_object")]
    #[inline(always)]
    fn is_mmtk_object(&self, _addr: Address) -> bool {
        false
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        panic!(
            "Called initialize_object_metadata() on {:x}, which maps to an empty space",
            object
        )
    }

    fn sft_trace_object(
        &self,
        _queue: &mut VectorObjectQueue,
        object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        // We do not have the `VM` type parameter here, so we cannot forward the call to the VM.
        panic!(
            "Call trace_object() on {} (chunk {}), which maps to an empty space. SFTProcessEdges does not support the fallback to vm_trace_object().",
            object,
            conversions::chunk_align_down(object.to_address()),
        )
    }
}
