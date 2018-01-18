use ::plan::plan_constraints::PlanConstraints;

pub struct SSConstraints {}

impl PlanConstraints for SSConstraints {
    fn moves_objects() -> bool {
        true
    }

    fn gc_header_bits() -> usize {
        2
    }

    fn gc_header_words() -> usize {
        0
    }

    fn num_specialized_scans() -> usize {
        1
    }
}
