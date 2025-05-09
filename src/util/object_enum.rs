//! Helper types for object enumeration

use std::marker::PhantomData;

use crate::vm::VMBinding;

use super::{
    heap::{chunk_map::ChunkMap, MonotonePageResource},
    linear_scan::Region,
    metadata::{side_metadata::spec_defs::VO_BIT, vo_bit},
    Address, ObjectReference,
};

/// A trait for enumerating objects in spaces, used by [`Space::enumerate_objects`].
///
/// [`Space::enumerate_objects`]: crate::policy::space::Space::enumerate_objects
pub trait ObjectEnumerator {
    /// Visit a single object.
    fn visit_object(&mut self, object: ObjectReference);
    /// Visit an address range that may contain objects.
    fn visit_address_range(&mut self, start: Address, end: Address);
}

/// An implementation of `ObjectEnumerator` that wraps a callback.
pub struct ClosureObjectEnumerator<F, VM>
where
    F: FnMut(ObjectReference),
    VM: VMBinding,
{
    object_callback: F,
    phantom_data: PhantomData<VM>,
}

impl<F, VM> ClosureObjectEnumerator<F, VM>
where
    F: FnMut(ObjectReference),
    VM: VMBinding,
{
    pub fn new(object_callback: F) -> Self {
        Self {
            object_callback,
            phantom_data: PhantomData,
        }
    }
}

impl<F, VM> ObjectEnumerator for ClosureObjectEnumerator<F, VM>
where
    F: FnMut(ObjectReference),
    VM: VMBinding,
{
    fn visit_object(&mut self, object: ObjectReference) {
        (self.object_callback)(object);
    }

    fn visit_address_range(&mut self, start: Address, end: Address) {
        VO_BIT.scan_non_zero_values::<u8>(start, end, &mut |address| {
            let object = vo_bit::get_object_ref_for_vo_addr(address);
            (self.object_callback)(object);
        })
    }
}

/// Allow querying if a block may have objects. `MarkSweepSpace` and `ImmixSpace` use different
/// `Block` types, and they have different block states. This trait lets both `Block` types provide
/// the same `may_have_objects` method.
pub(crate) trait BlockMayHaveObjects: Region {
    /// Return `true` if the block may contain valid objects (objects with the VO bit set). Return
    /// `false` otherwise.
    ///
    /// This function is used during object enumeration to filter out memory regions that do not
    /// contain objects. Because object enumeration may happen at mutator time, and another mutators
    /// may be allocating objects, this function only needs to reflect the state of the block at the
    /// time of calling, and is allowed to conservatively return `true`.
    fn may_have_objects(&self) -> bool;
}

pub(crate) fn enumerate_blocks_from_chunk_map<B>(
    enumerator: &mut dyn ObjectEnumerator,
    chunk_map: &ChunkMap,
) where
    B: BlockMayHaveObjects,
{
    for chunk in chunk_map.all_chunks() {
        for block in chunk.iter_region::<B>() {
            if block.may_have_objects() {
                enumerator.visit_address_range(block.start(), block.end());
            }
        }
    }
}

pub(crate) fn enumerate_blocks_from_monotonic_page_resource<VM>(
    enumerator: &mut dyn ObjectEnumerator,
    pr: &MonotonePageResource<VM>,
) where
    VM: VMBinding,
{
    for (start, size) in pr.iterate_allocated_regions() {
        enumerator.visit_address_range(start, start + size);
    }
}
