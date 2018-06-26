use ::plan::TraceLocal;
use ::util::ObjectReference;
use ::vm::References;

use ::vm::jikesrvm::entrypoint::*;
use super::JTOC_BASE;

pub struct VMReferences {}

impl References for VMReferences {
    fn forward_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        let trace_ptr = trace as *mut T;
        unsafe {
            jtoc_call!(PROCESS_REFERENCE_TYPES_METHOD_OFFSET, thread_id, trace_ptr, false);
        }
    }

    fn scan_weak_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        let trace_ptr = trace as *mut T;
        unsafe {
            jtoc_call!(SCAN_WEAK_REFERENCE_TYPE_METHOD_OFFSET, thread_id, trace_ptr, false);
        }
    }

    fn scan_soft_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        let trace_ptr = trace as *mut T;
        unsafe {
            jtoc_call!(SCAN_SOFT_REFERENCE_TYPE_METHOD_OFFSET, thread_id, trace_ptr, false);
        }
    }

    fn scan_phantom_refs<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        let trace_ptr = trace as *mut T;
        unsafe {
            jtoc_call!(SCAN_PHANTOM_REFERENCE_TYPE_METHOD_OFFSET, thread_id, trace_ptr, false);
        }
    }
}