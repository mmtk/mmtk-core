use crate::util::constants::*;

/// This struct defines plan-specific constraints.
/// Most of the constraints are constants. Each plan should declare a constant of this struct,
/// and use the constant wherever possible. However, for plan-neutral implementations,
/// these constraints are not constant.
pub struct PlanConstraints {
    pub moves_objects: bool,
    pub gc_header_bits: usize,
    pub gc_header_words: usize,
    pub num_specialized_scans: usize,
    pub max_non_los_copy_bytes: usize,
    pub needs_write_barrier: bool,
    pub needs_log_bit_in_header: bool,
    pub needs_log_bit_in_header_num: usize,
    // the following seems unused for now
    pub needs_linear_scan: bool,
    pub needs_concurrent_workers: bool,
    pub generate_gc_trace: bool,
    pub needs_forward_after_liveness: bool,
}

impl PlanConstraints {
    pub const fn default() -> Self {
        PlanConstraints {
            moves_objects: false,
            gc_header_bits: 0,
            gc_header_words: 0,
            num_specialized_scans: 0,
            max_non_los_copy_bytes: MAX_INT,
            needs_write_barrier: false,
            needs_log_bit_in_header: false,
            needs_log_bit_in_header_num: 0,
            needs_linear_scan: SUPPORT_CARD_SCANNING || LAZY_SWEEP,
            needs_concurrent_workers: false,
            generate_gc_trace: false,
            needs_forward_after_liveness: false,
        }
    }
}
