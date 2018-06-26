use ::plan::TraceLocal;
use ::util::ObjectReference;
use ::vm::References;

pub struct VMReferences {}

impl References for VMReferences {
    fn forward_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn scan_weak_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn scan_soft_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }

    fn scan_phantom_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unimplemented!()
    }
}