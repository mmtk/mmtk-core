use ::plan::TraceLocal;
use ::util::ObjectReference;

pub trait References {
    fn forward_refs<T: TraceLocal>(trace: &mut T, thread_id: usize);
    fn scan_weak_refs<T: TraceLocal>(trace: &mut T, thread_id: usize);
    fn scan_soft_refs<T: TraceLocal>(trace: &mut T, thread_id: usize);
    fn scan_phantom_refs<T: TraceLocal>(trace: &mut T, thread_id: usize);
}