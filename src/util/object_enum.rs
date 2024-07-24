//! Helper types for object enumeration

use std::marker::PhantomData;

use crate::vm::VMBinding;

use super::{
    heap::{
        chunk_map::{ChunkMap, ChunkState},
        MonotonePageResource,
    }, linear_scan::Region, metadata::side_metadata::spec_defs::VO_BIT, Address, ObjectReference
};

/// A trait for enumerating objects in spaces.
///
/// This is a trait object type, so we avoid using generics.  Because this trait may be used as a
/// `&mut dyn`, we avoid the cost of dynamic dispatching by allowing the user to supply an address
/// range instead of a single object reference.  The implementor of this trait will use linear
/// scanning to find objects in the range in batch.  But if the space is too sparse (e.g. LOS) and
/// the cost of linear scanning is greater than the dynamic dispatching, use `visit_object`
/// directly.
pub trait ObjectEnumerator {
    /// Visit a single object.
    fn visit_object(&mut self, object: ObjectReference);
    /// Visit an address range that may contain objects.
    fn visit_address_range(&mut self, addr_range: std::ops::Range<Address>);
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

    fn visit_address_range(&mut self, addr_range: std::ops::Range<Address>) {
        VO_BIT.scan_non_zero_values::<u8>(addr_range.start, addr_range.end, |address| {
            let object = ObjectReference::from_address::<VM>(address);
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
        if chunk_map.get(chunk) == ChunkState::Allocated {
            for block in chunk.iter_region::<B>() {
                if block.may_have_objects() {
                    enumerator.visit_address_range(block.as_range());
                }
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
        enumerator.visit_address_range(start..(start + size));
    }
}
