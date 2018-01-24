use ::vm::Scanning;
use ::plan::{TransitiveClosure, TraceLocal, MutatorContext, Plan, SelectedPlan, ParallelCollector};
use ::util::{ObjectReference, Address, SynchronizedCounter};
use ::vm::jikesrvm::jtoc::*;
use super::JTOC_BASE;
use super::unboxed_size_constants::LOG_BYTES_IN_ADDRESS;
use super::super::VMObjectModel;
use super::super::ObjectModel;
use super::scheduling::VMScheduling;
use std::mem::size_of;
use std::slice;

static COUNTER: SynchronizedCounter = SynchronizedCounter::new(0);

pub struct VMScanning {}

impl Scanning for VMScanning {
    fn scan_object<T: TransitiveClosure>(trace: &mut T, object: ObjectReference, thread_id: usize) {
        debug!("jtoc_call");
        let obj_ptr = object.value();
        let elt0_ptr: usize = unsafe {
            jtoc_call!(GET_OFFSET_ARRAY_METHOD_OFFSET, thread_id, obj_ptr)
        };
        debug!("elt0_ptr: {}", elt0_ptr);
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

    fn notify_initial_thread_scan_complete(partial_scan: bool, thread_id: usize) {
        if !partial_scan {
            unsafe {
                jtoc_call!(SNIP_OBSOLETE_COMPILED_METHODS_METHOD_OFFSET, thread_id);
            }
        }

        // FIXME: This should really be called on a specific mutator,
        //        but since we're not dealing with write barriers for
        //        now we'll ignore it.
        <SelectedPlan as Plan>::MutatorT::flush_remembered_sets();
    }

    fn compute_static_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        super::scan_statics::scan_statics(trace, thread_id);
    }

    fn compute_global_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        unsafe {
            let thread = VMScheduling::thread_from_id(thread_id);
            let system_thread = Address::from_usize(
                (thread + SYSTEM_THREAD_FIELD_OFFSET).load::<usize>());
            let cc = &*((system_thread + WORKER_INSTANCE_FIELD_OFFSET)
                .load::<*const <SelectedPlan as Plan>::CollectorT>());

            let jni_functions = JTOC_BASE + JNI_FUNCTIONS_FIELD_OFFSET;
            let jni_function_table_usize = (jni_functions + JNI_FUNCTIONS_FIELD_OFFSET).load::<usize>();
            let jni_function_table = Address::from_usize(jni_function_table_usize);
            let threads = cc.parallel_worker_count();
            let mut size = jikesrvm_instance_call!(jni_function_table, FUNCTION_TABLE_LENGTH_METHOD_OFFSET,
                thread_id, jni_function_table_usize);
            let mut chunk_size = size / threads;

            let mut start = cc.parallel_worker_ordinal() * chunk_size;
            let mut end = if cc.parallel_worker_ordinal() + 1 == threads {
                size
            } else {
                threads * chunk_size
            };

            for i in start .. end {
                let function_address_slot = jni_functions + (i * 4);
                if jtoc_call!(IMPLEMENTED_IN_JAVA_METHOD_OFFSET, thread_id, i) != 0 {
                    trace.process_root_edge(function_address_slot, true);
                } else {
                    // Function implemented as a C function, must not be
                    // scanned.
                }
            }

            let linkage_triplets = Address::from_usize(
                (JTOC_BASE + LINKAGE_TRIPLETS_FIELD_OFFSET).load::<usize>());
            if !linkage_triplets.is_zero() {
                for i in start .. end {
                    trace.process_root_edge(linkage_triplets + i * 4, true);
                }
            }

            let jni_global_refs = Address::from_usize(
                (JTOC_BASE + JNI_GLOBAL_REFS_FIELD2_OFFSET).load::<usize>());
            size = (jni_global_refs - 4).load::<usize>();
            chunk_size = size / threads;
            start = cc.parallel_worker_ordinal() * chunk_size;
            end = if cc.parallel_worker_ordinal() + 1 == threads {
                size
            } else {
                threads * chunk_size
            };

            for i in start .. end {
                trace.process_root_edge(jni_global_refs + 4 * i, true);
            }
        }
    }

    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        Self::compute_thread_roots(trace, false, thread_id)
    }

    fn compute_new_thread_roots<T: TraceLocal>(trace: &mut T, thread_id: usize) {
        Self::compute_thread_roots(trace, true, thread_id)
    }

    fn compute_bootimage_roots<T: TraceLocal>(trace: &mut T) {
        unimplemented!()
    }

    fn supports_return_barrier() -> bool {
        // FIXME: Really?
        cfg!(target_arch = "x86")
    }
}

impl VMScanning {
    fn compute_thread_roots<T: TraceLocal>(trace: &mut T, new_roots_sufficient: bool, thread_id: usize) {
        unsafe {
            let process_code_locations =
                (JTOC_BASE + MOVES_CODE_FIELD_OFFSET).load::<bool>();

            let num_threads =
                (JTOC_BASE + NUM_THREADS_FIELD_OFFSET).load::<usize>();

            loop {
                let thread_index = COUNTER.increment();
                if thread_index > num_threads {
                    break;
                }

                let thread = VMScheduling::thread_from_index(thread_index);

                if thread.is_zero() {
                    continue;
                }

                if (thread + IS_COLLECTOR_FIELD_OFFSET).load::<bool>() {
                    continue;
                }

                let trace_ptr = trace as *mut T;
                let thread_usize = thread.as_usize();
                jtoc_call!(SCAN_THREAD_METHOD_OFFSET, thread_id, thread_usize, trace_ptr,
                    new_roots_sufficient);
            }
        }
    }
}