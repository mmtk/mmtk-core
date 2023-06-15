//! This module updates of VO bits during GC.  It is used for spaces that do not clear the metadata
//! of some dead objects during GC.  Currently, only ImmixSpace is impacted.
//!
//! | Policy            | When are VO bits of dead objects cleared                      |
//! |-------------------|---------------------------------------------------------------|
//! | MarkSweepSpace    | When sweeping cells of dead objects                           |
//! | MarkCompactSpace  | When compacting                                               |
//! | CopySpace         | When releasing the space                                      |
//!
//! For ImmixSpace, if a line contains both live and dead objects, live objects will be traced,
//! but dead objects will not be visited.  Therefore we cannot clear the VO bits of individual
//! dead objects.  We cannot clear all VO bits for the line in bulk because it contains live
//! objects.  This module updates the VO bits for such regions (e.g. Immix lines, or Immix blocks
//! if Immix is configured to be block-only).
//!
//! We implement several strategies depending on whether mmtk-core or the VM binding also requires
//! the VO bits to also be available during tracing, for the purpose of
//!
//! -   conservative stack scanning
//! -   supporting interior pointers
//! -   heap dumping and object graph validation
//! -   sanity check
//!
//! The handling is very sensitive to `VOBitUpdateStrategy`, and may be a bit verbose.
//! We abstract VO-bit-related code out of the main GC algorithms (such as Immix) to make it more
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
    vm::{ObjectModel, VMBinding},
    MMTK,
};

/// The strategy to update the valid object (VO) bits metadata for some spaces.
///
/// Note that some strategies have implications on the availability of VO bits and the layout of
/// the mark bits metadata.  VM bindings should choose the appropriate strategy according to its
/// specific needs.
#[derive(Debug)]
pub enum VOBitUpdateStrategy {
    /// Clear all VO bits after stacks are scanned, and reconstruct the VO bits during tracing.
    ///
    /// This strategy is the default because it has minimum overhead.  If the VM does not have any
    /// special requirements other than conservative stack scanning, it should use this strategy.
    ///
    /// The main limitation is that the VO bits metadata is not available during tracing, because
    /// it is cleared after stack scanning.  If the VM needs to use the
    /// [`crate::memory_manager::is_mmtk_object`] function during tracing (for example, if some
    /// *fields* are conservative), it cannot use this strategy.
    ///
    /// This strategy is described in the paper *Fast Conservative Garbage Collection* published
    /// in OOPSLA'14.  See: <https://dl.acm.org/doi/10.1145/2660193.2660198>
    ClearAndReconstruct,
    /// Copy the mark bits metadata over to the VO bits metadata after tracing.
    ///
    /// This strategy will keep the VO bits metadata available during tracing.  However, it
    /// requires the mark bits to be on the side.  The VM cannot use this strategy if it uses
    /// in-header mark bits.
    CopyFromMarkBits,
}

impl VOBitUpdateStrategy {
    /// Return `true` if the VO bit metadata is available during tracing.
    pub fn vo_bit_available_during_tracing(&self) -> bool {
        match *self {
            VOBitUpdateStrategy::ClearAndReconstruct => false,
            VOBitUpdateStrategy::CopyFromMarkBits => true,
        }
    }
}

const fn strategy<VM: VMBinding>() -> VOBitUpdateStrategy {
    // TODO: Select strategy wisely if we add features for heap dumping or interior reference.
    if VM::VMObjectModel::NEED_VO_BITS_DURING_TRACING {
        VOBitUpdateStrategy::CopyFromMarkBits
    } else {
        VOBitUpdateStrategy::ClearAndReconstruct
    }
}

pub(crate) fn validate_config<VM: VMBinding>() {
    let s = strategy::<VM>();
    match s {
        VOBitUpdateStrategy::ClearAndReconstruct => {
            // Always valid
        }
        VOBitUpdateStrategy::CopyFromMarkBits => {
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

pub(crate) fn schedule_clear_vo_bits_packets_if_needed<VM: VMBinding>(
    chunk_map: &ChunkMap,
    scheduler: &GCWorkScheduler<VM>,
) {
    match strategy::<VM>() {
        VOBitUpdateStrategy::ClearAndReconstruct => {
            // In this strategy, we clear all VO bits after stacks are scanned.
            let work_packets =
                chunk_map.generate_tasks(|chunk| Box::new(ClearVOBitsAfterPrepare { chunk }));
            scheduler.work_buckets[WorkBucketStage::ClearVOBits].bulk_add(work_packets);
        }
        VOBitUpdateStrategy::CopyFromMarkBits => {
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
        VOBitUpdateStrategy::ClearAndReconstruct => {
            // In this strategy, we set the VO bit when an object is marked.
            vo_bit::set_vo_bit::<VM>(object);
        }
        VOBitUpdateStrategy::CopyFromMarkBits => {
            // VO bit was not cleared before tracing in this strategy.  Do nothing.
        }
    }
}

pub(crate) fn on_object_forwarded<VM: VMBinding>(new_object: ObjectReference) {
    match strategy::<VM>() {
        VOBitUpdateStrategy::ClearAndReconstruct => {
            // In this strategy, we set the VO bit of the to-space object when forwarded.
            vo_bit::set_vo_bit::<VM>(new_object);
        }
        VOBitUpdateStrategy::CopyFromMarkBits => {
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

pub(crate) fn on_region_swept<VM: VMBinding, R: Region>(region: &R, is_occupied: bool) {
    match strategy::<VM>() {
        VOBitUpdateStrategy::ClearAndReconstruct => {
            // Do nothing.  The VO bit metadata is already reconstructed.
        }
        VOBitUpdateStrategy::CopyFromMarkBits => {
            // In this strategy, we need to update the VO bits state after marking.
            if is_occupied {
                // If the block has live objects, copy the VO bits from mark bits.
                vo_bit::bcopy_vo_bit_from_mark_bit::<VM>(region.start(), R::BYTES);
            } else {
                // If the block has no live objects, simply clear the VO bits.
                vo_bit::bzero_vo_bit(region.start(), R::BYTES);
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
