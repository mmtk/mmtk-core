use ::plan::plan_constraints::PlanConstraints;

pub struct NoGCConstraints {}

impl PlanConstraints for NoGCConstraints {
    fn moves_objects() -> bool {
        false
    }

    fn gc_header_bits() -> usize {
        0
    }

    fn gc_header_words() -> usize {
        0
    }

    fn num_specialized_scans() -> usize {
        0
    }
}