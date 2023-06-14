//! This module updates of VO bits for ImmixSpace during GC.
//! The handling is very sensitive to `ImmixVOBitUpdateStrategy`, and may be a bit verbose.
//! We abstract VO-bit-related code out of the main parts of the Immix algorithm to make it more
//! readable.

use atomic::Ordering;

use crate::{
    scheduler::{GCWork, GCWorkScheduler, GCWorker, WorkBucketStage},
    util::{
        heap::chunk_map::{Chunk, ChunkMap},
        linear_scan::Region,
        metadata::vo_bit,
        ObjectReference,
    },
    vm::{ImmixVOBitUpdateStrategy, ObjectModel, VMBinding},
    MMTK,
};

use super::block::Block;

const fn strategy<VM: VMBinding>() -> ImmixVOBitUpdateStrategy {
    VM::VMObjectModel::IMMIX_VO_BIT_UPDATE_STRATEGY
}

pub(crate) fn validate_config<VM: VMBinding>() {
    let s = strategy::<VM>();
    match s {
        ImmixVOBitUpdateStrategy::ClearAndReconstruct => {
            // Always valid
        }
        ImmixVOBitUpdateStrategy::CopyFromMarkBits => {
            let mark_bit_spec = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC;
            assert!(
                mark_bit_spec.is_on_side(),
                "The {s:?} strategy requires the mark bits to be on the side."
            );

            let mark_bit_meta = mark_bit_spec.extract_side_spec();
            let vo_bit_meta = vo_bit::VO_BIT_SIDE_METADATA_SPEC;

            assert_eq!(
                mark_bit_meta.log_bytes_in_region,
                vo_bit_meta.log_bytes_in_region,
                "The {s:?} strategy requires the mark bits to have the same granularity as the VO bits."
            );
            assert_eq!(mark_bit_meta.log_num_of_bits, vo_bit_meta.log_num_of_bits,
                "The {s:?} strategy requires the mark bits to have the same number of bits per object as the VO bits.");
        }
    }
}

pub(crate) fn prepare_extra_packets<VM: VMBinding>(
    chunk_map: &ChunkMap,
    scheduler: &GCWorkScheduler<VM>,
) {
    match strategy::<VM>() {
        ImmixVOBitUpdateStrategy::ClearAndReconstruct => {
            // In this strategy, we clear all VO bits after stacks are scanned.
            let work_packets =
                chunk_map.generate_tasks(|chunk| Box::new(ClearVOBitsAfterPrepare { chunk }));
            scheduler.work_buckets[WorkBucketStage::ClearVOBits].bulk_add(work_packets);
        }
        ImmixVOBitUpdateStrategy::CopyFromMarkBits => {
            // Do nothing.
        }
    }
}

pub(crate) fn on_trace_object<VM: VMBinding>(object: ObjectReference) {
    if strategy::<VM>().vo_bit_available_during_tracing() {
        // If the VO bits are available during tracing,
        // we validate the objects we trace using the VO bits.
        debug_assert!(
            vo_bit::is_vo_bit_set::<VM>(object),
            "{:x}: VO bit not set",
            object
        );
    }
}

pub(crate) fn on_object_marked<VM: VMBinding>(object: ObjectReference) {
    match strategy::<VM>() {
        ImmixVOBitUpdateStrategy::ClearAndReconstruct => {
            // In this strategy, we set the VO bit when an object is marked.
            vo_bit::set_vo_bit::<VM>(object);
        }
        ImmixVOBitUpdateStrategy::CopyFromMarkBits => {
            // VO bit was not cleared before tracing in this strategy.  Do nothing.
        }
    }
}

pub(crate) fn on_object_forwarded<VM: VMBinding>(new_object: ObjectReference) {
    match strategy::<VM>() {
        ImmixVOBitUpdateStrategy::ClearAndReconstruct => {
            // In this strategy, we set the VO bit of the to-space object when forwarded.
            vo_bit::set_vo_bit::<VM>(new_object);
        }
        ImmixVOBitUpdateStrategy::CopyFromMarkBits => {
            // In this strategy, we will copy mark bits to VO bits.
            // We need to set mark bits for to-space objects, too.
            VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(
                new_object,
                1,
                None,
                Ordering::SeqCst,
            );

            // In theory, we don't need to set the VO bit for to-space objects because we
            // will copy the VO bits from mark bits during Release.  However, Some VMs
            // allow the same edge to be traced twice, and MMTk will see the edge pointing
            // to a to-space object when visiting the edge the second time.  Considering
            // that we may want to use the VO bits to validate if the edge is valid, we set
            // the VO bit for the to-space object, too.
            vo_bit::set_vo_bit::<VM>(new_object);
        }
    }
}

pub(crate) fn on_block_swept<VM: VMBinding>(block: &Block, is_occupied: bool) {
    match strategy::<VM>() {
        ImmixVOBitUpdateStrategy::ClearAndReconstruct => {
            // Do nothing.  The VO bit metadata is already reconstructed.
        }
        ImmixVOBitUpdateStrategy::CopyFromMarkBits => {
            // In this strategy, we need to update the VO bits state after marking.
            if is_occupied {
                // If the block has live objects, copy the VO bits from mark bits.
                vo_bit::bcopy_vo_bit_from_mark_bit::<VM>(block.start(), Block::BYTES);
            } else {
                // If the block has no live objects, simply clear the VO bits.
                vo_bit::bzero_vo_bit(block.start(), Block::BYTES);
            }
        }
    }
}

/// A work packet to clear VO bit metadata after Prepare.
pub struct ClearVOBitsAfterPrepare {
    pub chunk: Chunk,
}

impl<VM: VMBinding> GCWork<VM> for ClearVOBitsAfterPrepare {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        vo_bit::bzero_vo_bit(self.chunk.start(), Chunk::BYTES);
    }
}
