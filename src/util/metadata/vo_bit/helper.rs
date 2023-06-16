//! This module updates of VO bits during GC.  It is used for spaces that do not clear the metadata
//! of some dead objects during GC.  Currently, only ImmixSpace is affected.
//!
//! | Policy            | When are VO bits of dead objects cleared                      |
//! |-------------------|---------------------------------------------------------------|
//! | MarkSweepSpace    | when sweeping cells of dead objects                           |
//! | MarkCompactSpace  | when compacting                                               |
//! | CopySpace         | when releasing the space                                      |
//!
//! The policies listed above trivially clear the VO bits for dead objects (individually or in
//! bulk), and make the VO bits available during tracing.
//!
//! For ImmixSpace, if a line contains both live and dead objects, live objects will be traced,
//! but dead objects will not be visited.  Therefore we cannot clear the VO bits of individual
//! dead objects.  We cannot clear all VO bits for the line in bulk because it contains live
//! objects.  This module updates the VO bits for such regions (e.g. Immix lines, or Immix blocks
//! if Immix is configured to be block-only).
//!
//! We implement several strategies depending on whether mmtk-core or the VM binding also requires
//! the VO bits to also be available during tracing.
//!
//! The handling is very sensitive to `VOBitUpdateStrategy`, and may be a bit verbose.
//! We abstract VO-bit-related code out of the main GC algorithms (such as Immix) to make it more
//! readable.

use atomic::Ordering;

use crate::{
    util::{linear_scan::Region, metadata::vo_bit, ObjectReference},
    vm::{ObjectModel, VMBinding},
};

/// The strategy to update the valid object (VO) bits.
///
/// Each stategy has its strength and limitation.  We should choose a strategy according to the
/// configuration of the VM binding.  See [`strategy`].
#[derive(Debug)]
enum VOBitUpdateStrategy {
    /// Clear all VO bits after stacks are scanned, and reconstruct the VO bits during tracing.
    ///
    /// Pros:
    /// -   Minimum overhead.
    ///
    /// Cons:
    /// -   VO bits are not available during tracing.
    ///
    /// This strategy is described in the paper *Fast Conservative Garbage Collection* published
    /// in OOPSLA'14.  See: <https://dl.acm.org/doi/10.1145/2660193.2660198>
    ClearAndReconstruct,
    /// Copy the mark bits metadata over to the VO bits metadata after tracing.
    ///
    /// Pros:
    /// -   VO bits are available during tracing.
    ///
    /// Cons:
    /// -   Requires marking bits to be on the side.
    /// -   Has extra time overhead.
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

/// Select a strategy for the VM.  It is a `const` function so it always returns the same strategy
/// for a given VM.
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

pub(crate) fn need_to_clear_vo_bits_before_tracing<VM: VMBinding>() -> bool {
    match strategy::<VM>() {
        VOBitUpdateStrategy::ClearAndReconstruct => true,
        VOBitUpdateStrategy::CopyFromMarkBits => false,
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

            // We set the VO bit for the to-space object eagerly.
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
