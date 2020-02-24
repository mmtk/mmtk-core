use super::memory_manager_constants::*;
use super::java_header_constants::*;
use super::scan_sanity;

use ::vm::Scanning;
use ::plan::{TransitiveClosure, TraceLocal, MutatorContext, Plan, SelectedPlan, ParallelCollector};
use ::util::{ObjectReference, Address, SynchronizedCounter};
use ::vm::jikesrvm::entrypoint::*;
use super::JTOC_BASE;
use super::super::unboxed_size_constants::LOG_BYTES_IN_ADDRESS;
use vm::jikesrvm::object_model::VMObjectModel;
use vm::ObjectModel;
use vm::jikesrvm::active_plan::VMActivePlan;
use vm::ActivePlan;
use vm::jikesrvm::collection::VMCollection;
use vm::Collection;
use std::mem::size_of;
use std::slice;
use ::vm::jikesrvm::java_header::TIB_OFFSET;
use ::vm::jikesrvm::tib_layout_constants::TIB_TYPE_INDEX;
use ::vm::unboxed_size_constants::BYTES_IN_ADDRESS;
use ::util::OpaquePointer;

use libc::c_void;
use vm::jikesrvm::JikesRVM;

static COUNTER: SynchronizedCounter = SynchronizedCounter::new(0);

pub struct VMScanning {}

const DUMP_REF: bool = false;

impl Scanning<JikesRVM> for VMScanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference, tls: OpaquePointer) {
        if DUMP_REF {
            let obj_ptr = object.value();
            unsafe { jtoc_call!(DUMP_REF_METHOD_OFFSET, tls, obj_ptr); }
        }
        trace!("Getting reference array");
        let elt0_ptr: usize = unsafe {
            let rvm_type = VMObjectModel::load_rvm_type(object);
            (rvm_type + REFERENCE_OFFSETS_FIELD_OFFSET).load::<usize>()
        };
        trace!("elt0_ptr: {}", elt0_ptr);
        // In a primitive array this field points to a zero-length array.
        // In a reference array this field is null.
        // In a class with pointers, it contains the offsets of reference-containing instance fields
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
        COUNTER.reset();
    }

    fn notify_initial_thread_scan_complete(partial_scan: bool, tls: OpaquePointer) {
        if !partial_scan {
            unsafe {
                jtoc_call!(SNIP_OBSOLETE_COMPILED_METHODS_METHOD_OFFSET, tls);
            }
        }

        unsafe {
            VMActivePlan::mutator(tls).flush_remembered_sets();
        }
    }

    fn compute_static_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        super::scan_statics::scan_statics(trace, tls);
    }

    fn compute_global_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        unsafe {
            let cc = VMActivePlan::collector(tls);

            let jni_functions = (JTOC_BASE + JNI_FUNCTIONS_FIELD_OFFSET).load::<Address>();
            trace!("jni_functions: {:?}", jni_functions);

            let threads = cc.parallel_worker_count();
            // @Intrinsic JNIFunctions.length()
            let mut size = (jni_functions + ARRAY_LENGTH_OFFSET).load::<usize>();
            trace!("size: {:?}", size);
            let mut chunk_size = size / threads;
            trace!("chunk_size: {:?}", chunk_size);
            let mut start = cc.parallel_worker_ordinal() * chunk_size;
            trace!("start: {:?}", start);
            let mut end = if cc.parallel_worker_ordinal() + 1 == threads {
                size
            } else {
                threads * chunk_size
            };
            trace!("end: {:?}", end);

            for i in start..end {
                let function_address_slot = jni_functions + (i << LOG_BYTES_IN_ADDRESS);
                if jtoc_call!(IMPLEMENTED_IN_JAVA_METHOD_OFFSET, tls, i) != 0 {
                    trace!("function implemented in java {:?}", function_address_slot);
                    trace.process_root_edge(function_address_slot, true);
                } else {
                    // Function implemented as a C function, must not be
                    // scanned.
                }
            }

            let linkage_triplets = (JTOC_BASE + LINKAGE_TRIPLETS_FIELD_OFFSET).load::<Address>();
            if !linkage_triplets.is_zero() {
                for i in start..end {
                    trace.process_root_edge(linkage_triplets + i * 4, true);
                }
            }

            let jni_global_refs = (JTOC_BASE + JNI_GLOBAL_REFS_FIELD2_OFFSET).load::<Address>();
            trace!("jni_global_refs address: {:?}", jni_global_refs);
            size = (jni_global_refs - 4).load::<usize>();
            trace!("jni_global_refs size: {:?}", size);
            chunk_size = size / threads;
            trace!("chunk_size: {:?}", chunk_size);
            start = cc.parallel_worker_ordinal() * chunk_size;
            trace!("start: {:?}", start);
            end = if cc.parallel_worker_ordinal() + 1 == threads {
                size
            } else {
                threads * chunk_size
            };
            trace!("end: {:?}", end);

            for i in start..end {
                trace.process_root_edge(jni_global_refs + (i << LOG_BYTES_IN_ADDRESS), true);
            }
        }
    }

    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        Self::compute_thread_roots(trace, false, tls)
    }

    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        Self::compute_thread_roots(trace, true, tls)
    }

    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T, tls: OpaquePointer) {
        super::scan_boot_image::scan_boot_image(trace, tls);
    }

    fn supports_return_barrier() -> bool {
        // FIXME: Really?
        cfg!(target_arch = "x86")
    }
}

impl VMScanning {
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, new_roots_sufficient: bool, tls: OpaquePointer) {
        unsafe {
            let process_code_locations = MOVES_CODE;

            let num_threads =
                (JTOC_BASE + NUM_THREADS_FIELD_OFFSET).load::<usize>();

            loop {
                let thread_index = COUNTER.increment();
                if thread_index > num_threads {
                    break;
                }

                let thread = VMCollection::thread_from_index(thread_index);

                if thread.is_zero() {
                    continue;
                }

                if (thread + IS_COLLECTOR_FIELD_OFFSET).load::<bool>() {
                    continue;
                }

                let trace_ptr = trace as *mut T;
                let thread_usize = thread.as_usize();
                debug!("Calling JikesRVM to compute thread roots, thread_usize={:x}", thread_usize);
                jtoc_call!(SCAN_THREAD_METHOD_OFFSET, tls, thread_usize, trace_ptr,
                    process_code_locations, new_roots_sufficient);
                debug!("Returned from JikesRVM thread roots");
            }
        }
    }
}