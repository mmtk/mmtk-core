//! Plan-specific constraints.

use crate::plan::barriers::BarrierSelector;
use crate::util::constants::*;

/// This struct defines plan-specific constraints.
/// Most of the constraints are constants. Each plan should declare a constant of this struct,
/// and use the constant wherever possible. However, for plan-neutral implementations,
/// these constraints are not constant.
pub struct PlanConstraints {
    /// Does the plan collect garbage? Obviously most plans do, but NoGC does not collect.
    pub collects_garbage: bool,
    /// True if the plan moves objects.
    pub moves_objects: bool,
    /// Size (in bytes) beyond which new regular objects must be allocated to the LOS.
    /// This usually depends on the restriction of the default allocator, e.g. block size for Immix,
    /// nursery size, max possible cell for freelist, etc.
    pub max_non_los_default_alloc_bytes: usize,
    /// Size (in bytes) beyond which copied objects must be copied to the LOS.
    /// This depends on the copy allocator.
    pub max_non_los_copy_bytes: usize,
    /// Does this plan use the log bit? See vm::ObjectModel::GLOBAL_LOG_BIT_SPEC.
    pub needs_log_bit: bool,
    /// Some plans may allow benign race for testing mark bit, and this will lead to trace the same edges
    /// multiple times. If a plan allows tracing duplicate edges, we will not run duplicate edge check
    /// in extreme_assertions.
    pub may_trace_duplicate_edges: bool,
    /// The barrier this plan uses. A binding may check this and know what kind of write barrier is in use
    /// if they would like to implement the barrier fast path in the binding side.
    pub barrier: BarrierSelector,
    // the following seems unused for now
    /// True if this plan requires linear scanning. This is unused and may be incorrect.
    pub needs_linear_scan: bool,
    /// True if this plan requires concurrent worker threads. This is unused and may be incorrect.
    pub needs_concurrent_workers: bool,
    /// Some policies do object forwarding after the first liveness transitive closure, such as mark compact.
    /// For plans that use those policies, they should set this as true.
    pub needs_forward_after_liveness: bool,
    /// Some (in fact, most) plans do nothing when preparing mutators before tracing (i.e. in
    /// `MutatorConfig::prepare_func`).  Those plans can set this to `false` so that the
    /// `PrepareMutator` work packets will not be created at all.
    pub needs_prepare_mutator: bool,
}

impl PlanConstraints {
    /// A const function to create the default plan constraints.
    pub const fn default() -> Self {
        PlanConstraints {
            collects_garbage: true,
            moves_objects: false,
            max_non_los_default_alloc_bytes: MAX_INT,
            max_non_los_copy_bytes: MAX_INT,
            // As `LAZY_SWEEP` is true, needs_linear_scan is true for all the plans. This is strange.
            // https://github.com/mmtk/mmtk-core/issues/1027 trackes the issue.
            needs_linear_scan: crate::util::constants::SUPPORT_CARD_SCANNING
                || crate::util::constants::LAZY_SWEEP,
            needs_concurrent_workers: false,
            may_trace_duplicate_edges: false,
            needs_forward_after_liveness: false,
            needs_log_bit: false,
            barrier: BarrierSelector::NoBarrier,
            needs_prepare_mutator: true,
        }
    }
}

/// The default plan constraints. Each plan should define their own plan contraints.
/// They can start from the default constraints and explicitly set some of the fields.
pub(crate) const DEFAULT_PLAN_CONSTRAINTS: PlanConstraints = PlanConstraints::default();

// Use 16 pages as the size limit for non-LOS objects to avoid copying large objects
pub const MAX_NON_LOS_ALLOC_BYTES_COPYING_PLAN: usize = 16 << LOG_BYTES_IN_PAGE;
