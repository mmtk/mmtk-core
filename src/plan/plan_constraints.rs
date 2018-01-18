pub trait PlanConstraints {
    fn moves_objects() -> bool;
    fn gc_header_bits() -> usize;
    fn gc_header_words() -> usize;
    fn num_specialized_scans() -> usize;
}