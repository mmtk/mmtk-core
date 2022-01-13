use crate::util::conversions;
use crate::util::conversions::*;
use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use crate::util::heap::layout::vm_layout_constants::MAX_CHUNKS;
use crate::util::Address;
use crate::util::ObjectReference;
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
    fn is_mmtk_object(&self, _object: ObjectReference) -> bool {
        true
    }
    /// Initialize object metadata (in the header, or in the side metadata).
    fn initialize_object_metadata(&self, object: ObjectReference, alloc: bool);
}

/// Print debug info for SFT. Should be false when committed.
const DEBUG_SFT: bool = cfg!(debug_assertions) && false;

#[derive(Debug, PartialEq, Eq)]
pub struct EmptySpaceSFT {}

const EMPTY_SFT_NAME: &str = "empty";

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
    fn is_mmtk_object(&self, _object: ObjectReference) -> bool {
        false
    }

    fn initialize_object_metadata(&self, object: ObjectReference, _alloc: bool) {
        panic!(
            "Called initialize_object_metadata() on {:x}, which maps to an empty space",
            object
        )
    }
}

// Create erased VM refs for each space type. In this way, we can store the refs in SFTMap (which
// cannot have a generic type parameter for VM, as it is a global variable)

use crate::util::erase_vm::define_erased_vm_ref;
define_erased_vm_ref!(ImmortalSpaceRef = super::immortalspace::ImmortalSpace<VM>);
define_erased_vm_ref!(CopySpaceRef = super::copyspace::CopySpace<VM>);
define_erased_vm_ref!(LargeObjectSpaceRef = super::largeobjectspace::LargeObjectSpace<VM>);
define_erased_vm_ref!(LockFreeImmortalSpaceRef = super::lockfreeimmortalspace::LockFreeImmortalSpace<VM>);
define_erased_vm_ref!(MarkCompactSpaceRef = super::markcompactspace::MarkCompactSpace<VM>);
define_erased_vm_ref!(MallocSpaceRef = super::mallocspace::MallocSpace<VM>);
define_erased_vm_ref!(ImmixSpaceRef = super::immix::ImmixSpace<VM>);

/// This enum helps dispatch SFT calls using a switch statement rather than virtual dispatch table.
/// The benefit is that with a switch statement, the call is static, thus can be inlined, which
/// may give us performance improvement.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SFTDispatch<'a> {
    ImmortalSpace(ImmortalSpaceRef<'a>),
    CopySpace(CopySpaceRef<'a>),
    LargeObjectSpace(LargeObjectSpaceRef<'a>),
    LockFreeImmortalSpace(LockFreeImmortalSpaceRef<'a>),
    MarkCompactSpace(MarkCompactSpaceRef<'a>),
    MallocSpace(MallocSpaceRef<'a>),
    ImmixSpace(ImmixSpaceRef<'a>),
    Empty(&'a EmptySpaceSFT),
}

/// This macro defines a given function, which forwards the call to the SFT implementation
/// depending on the enum type.
macro_rules! dispatch_sft_call {
    ($fn: tt = ($($args: tt: $tys: ty),*) -> $ret_ty: ty) => {
        #[inline(always)]
        pub fn $fn<VM: VMBinding>(&self, $($args: $tys),*) -> $ret_ty {
            match self {
                SFTDispatch::ImmortalSpace(r) => r.as_ref::<VM>().$fn($($args),*),
                SFTDispatch::CopySpace(r) => r.as_ref::<VM>().$fn($($args),*),
                SFTDispatch::LargeObjectSpace(r) => r.as_ref::<VM>().$fn($($args),*),
                SFTDispatch::LockFreeImmortalSpace(r) => r.as_ref::<VM>().$fn($($args),*),
                SFTDispatch::MarkCompactSpace(r) => r.as_ref::<VM>().$fn($($args),*),
                SFTDispatch::MallocSpace(r) => r.as_ref::<VM>().$fn($($args),*),
                SFTDispatch::ImmixSpace(r) => r.as_ref::<VM>().$fn($($args),*),
                SFTDispatch::Empty(r) => r.$fn($($args),*),
            }
        }
    }
}

#[allow(unused)]
impl<'a> SFTDispatch<'a> {
    dispatch_sft_call!(name = () -> &str);
    dispatch_sft_call!(get_forwarded_object = (object: ObjectReference) -> Option<ObjectReference>);
    dispatch_sft_call!(is_live = (object: ObjectReference) -> bool);
    dispatch_sft_call!(is_reachable = (object: ObjectReference) -> bool);
    dispatch_sft_call!(is_movable = () -> bool);
    #[cfg(feature = "sanity")]
    dispatch_sft_call!(is_sane = () -> bool);
    dispatch_sft_call!(is_mmtk_object = (object: ObjectReference) -> bool);
    dispatch_sft_call!(initialize_object_metadata = (object: ObjectReference, alloc: bool) -> ());
}

#[derive(Default)]
pub struct SFTMap<'a> {
    /// SFT table for SFT dispatch enum.
    sft: Vec<SFTDispatch<'a>>,
}

// TODO: MMTK<VM> holds a reference to SFTMap. We should have a safe implementation rather than use raw pointers for dyn SFT.
unsafe impl<'a> Sync for SFTMap<'a> {}

static EMPTY_SPACE_SFT: EmptySpaceSFT = EmptySpaceSFT {};

impl<'a> SFTMap<'a> {
    pub fn new() -> Self {
        SFTMap {
            // sft: vec![&EMPTY_SPACE_SFT; MAX_CHUNKS],
            sft: vec![SFTDispatch::Empty(&EMPTY_SPACE_SFT); MAX_CHUNKS],
        }
    }
    // This is a temporary solution to allow unsafe mut reference. We do not want several occurrence
    // of the same unsafe code.
    // FIXME: We need a safe implementation.
    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    /// Get the dyn SFT for the given address. Note that this returns a fat pointer for SFT,
    /// and dispatch on dyn SFT will be a dynamic dispatch.
    // pub fn get(&self, address: Address) -> &'a dyn SFT {
    //     let res = self.sft[address.chunk_index()];
    //     if DEBUG_SFT {
    //         trace!(
    //             "Get SFT for {} #{} = {}",
    //             address,
    //             address.chunk_index(),
    //             res.name()
    //         );
    //     }
    //     res
    // }

    /// Get the SFTDispatch for the given address. Note that this returns an enum for the SFT,
    /// and dispatch on this is static. However, the caller needs to know the <VM> type parameter
    /// in order to make the call.
    // This is unused for now.
    #[allow(unused)]
    #[inline(always)]
    pub fn get_dispatch(&self, address: Address) -> SFTDispatch {
        let res = self.sft[address.chunk_index()];
        if DEBUG_SFT {
            trace!(
                "Get SFT for {} #{} = {:?}",
                address,
                address.chunk_index(),
                res,
            );
        }
        res
    }

    fn log_update(&self, space: SFTDispatch, start: Address, bytes: usize) {
        debug!(
            "Update SFT for [{}, {}) as {:?}",
            start,
            start + bytes,
            space
        );
        let first = start.chunk_index();
        let last = conversions::chunk_align_up(start + bytes).chunk_index();
        let start_chunk = chunk_index_to_address(first);
        let end_chunk = chunk_index_to_address(last);
        debug!(
            "Update SFT for {} bytes of [{} #{}, {} #{})",
            bytes, start_chunk, first, end_chunk, last
        );
    }

    fn trace_sft_map(&self) {
        // For large heaps, it takes long to iterate each chunk. So check log level first.
        if log::log_enabled!(log::Level::Trace) {
            // print the entire SFT map
            const SPACE_PER_LINE: usize = 10;
            for i in (0..self.sft.len()).step_by(SPACE_PER_LINE) {
                let max = if i + SPACE_PER_LINE > self.sft.len() {
                    self.sft.len()
                } else {
                    i + SPACE_PER_LINE
                };
                let chunks: Vec<usize> = (i..max).collect();
                let spaces: Vec<String> = chunks.iter().map(|&x| format!("{:?}", self.sft[x])).collect();
                trace!("Chunk {}: {}", i, spaces.join(","));
            }
        }
    }

    /// Update SFT map for the given address range.
    /// It should be used when we acquire new memory and use it as part of a space. For example, the cases include:
    /// 1. when a space grows, 2. when initializing a contiguous space, 3. when ensure_mapped() is called on a space.
    pub fn update(
        &self,
        dispatch: SFTDispatch,
        start: Address,
        bytes: usize,
    ) {
        if DEBUG_SFT {
            self.log_update(dispatch, start, bytes);
        }
        let first = start.chunk_index();
        let last = conversions::chunk_align_up(start + bytes).chunk_index();
        for chunk in first..last {
            self.set(chunk, dispatch);
        }
        if DEBUG_SFT {
            self.trace_sft_map();
        }
    }

    // TODO: We should clear a SFT entry when a space releases a chunk.
    #[allow(dead_code)]
    pub fn clear(&self, chunk_start: Address) {
        assert!(chunk_start.is_aligned_to(BYTES_IN_CHUNK));
        let chunk_idx = chunk_start.chunk_index();
        self.set(
            chunk_idx,
            SFTDispatch::Empty(&EMPTY_SPACE_SFT),
        );
    }

    // Currently only used by 32 bits vm map
    #[allow(dead_code)]
    pub fn clear_by_index(&self, chunk_idx: usize) {
        self.set(
            chunk_idx,
            SFTDispatch::Empty(&EMPTY_SPACE_SFT),
        )
    }

    fn set(&self, chunk: usize, dispatch: SFTDispatch) {
        /*
         * This is safe (only) because a) this is only called during the
         * allocation and deallocation of chunks, which happens under a global
         * lock, and b) it only transitions from empty to valid and valid to
         * empty, so if there were a race to view the contents, in the one case
         * it would either see the new (valid) space or an empty space (both of
         * which are reasonable), and in the other case it would either see the
         * old (valid) space or an empty space, both of which are valid.
         */
        let self_mut = unsafe { self.mut_self() };
        // It is okay to set empty to valid, or set valid to empty. It is wrong if we overwrite a valid value with another valid value.
        if cfg!(debug_assertions) {
            let old = self_mut.sft[chunk];
            let new = dispatch;
            // Allow overwriting the same SFT pointer. E.g., if we have set SFT map for a space, then ensure_mapped() is called on the same,
            // in which case, we still set SFT map again.
            debug_assert!(
                matches!(old, SFTDispatch::Empty(_)) || matches!(new, SFTDispatch::Empty(_)) || old == new,
                "attempt to overwrite a non-empty chunk {} in SFT map (from {:?} to {:?})",
                chunk,
                old,
                new
            );
        }
        // self_mut.sft[chunk] = sft;
        self_mut.sft[chunk] = dispatch;
    }

    pub fn is_in_space<VM: VMBinding>(&self, object: ObjectReference) -> bool {
        if object.to_address().chunk_index() >= self.sft.len() {
            return false;
        }
        self.get_dispatch(object.to_address()).is_mmtk_object::<VM>(object)
    }
}
