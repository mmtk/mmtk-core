use ::vm::Scanning;
use ::plan::{TransitiveClosure, TraceLocal};
use ::util::{ObjectReference, Address};
use ::vm::jikesrvm::jtoc::*;
use ::vm::JTOC_BASE;
use ::vm::unboxed_size_constants::LOG_BYTES_IN_ADDRESS;
use ::vm::VMObjectModel;
use ::vm::object_model::ObjectModel;
use std::mem::size_of;
use std::slice;

pub struct VMScanning {}

const THREAD_PLACEHOLDER: usize = 1;

impl Scanning for VMScanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference) {
        // FIXME: pass the correct collector thread id
        let elt0_ptr: usize = jtoc_call!(GET_OFFSET_ARRAY_METHOD_JTOC_OFFSET, THREAD_PLACEHOLDER, object);
        if elt0_ptr == 0 {
            // object is a REFARRAY
            let length = VMObjectModel::get_array_length(object);
            for i in 0..length {
                trace.process_edge(object.to_address() + (i << LOG_BYTES_IN_ADDRESS));
            }
        } else {
            let len_ptr: usize = elt0_ptr - size_of::<isize>();
            let len = unsafe { *(len_ptr as *const isize) };
            let offsets = unsafe { slice::from_raw_parts(elt0_ptr as *const isize, len as usize) };

            for offset in offsets.iter() {
                trace.process_edge(object.to_address() + *offset);
            }
        }
    }

    fn reset_thread_counter() {
        unimplemented!()
    }

    fn notify_initial_thread_scan_complete(partial_scan: bool) {
        unimplemented!()
    }

    fn compute_static_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn compute_global_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn compute_thread_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        unimplemented!()
    }
}