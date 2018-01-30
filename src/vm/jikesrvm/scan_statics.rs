use super::entrypoint::*;
use super::JTOC_BASE;
use super::scheduling::VMScheduling;
use ::util::Address;
use ::plan::{TraceLocal, SelectedPlan, Plan, ParallelCollector};

use super::active_plan::VMActivePlan;
use super::super::ActivePlan;

static mut REF_SLOT_SIZE: usize = 0;
static mut CHUNK_SIZE_MASK: usize = 0;

pub unsafe fn set_ref_slot_size(thread_id: usize) {
    REF_SLOT_SIZE = jtoc_call!(GET_REFERENCE_SLOT_SIZE_METHOD_OFFSET, thread_id);
    CHUNK_SIZE_MASK = 0xFFFFFFFF - (REF_SLOT_SIZE - 1);
}

pub fn scan_statics<T: TraceLocal>(trace: &mut T, thread_id: usize) {
    unsafe {
        let slots = JTOC_BASE;
        let cc = VMActivePlan::collector(thread_id);

        let number_of_collectors: usize = cc.parallel_worker_count();
        let number_of_references: usize = jtoc_call!(GET_NUMBER_OF_REFERENCE_SLOTS_METHOD_OFFSET,
            thread_id);
        let chunk_size: usize = (number_of_references / number_of_collectors) & CHUNK_SIZE_MASK;
        let thread_ordinal = cc.parallel_worker_ordinal();

        let start: usize = if thread_ordinal == 0 {
            REF_SLOT_SIZE
        } else {
            thread_ordinal * chunk_size
        };

        let end: usize = if thread_ordinal + 1 == number_of_collectors {
            number_of_references
        } else {
            (thread_ordinal + 1) * chunk_size
        };

        let mut slot = start;
        while slot < end {
            let slot_offset = slot * 4;
            // TODO: check_reference?
            trace.process_root_edge(slots + slot_offset, true);
            slot += REF_SLOT_SIZE;
        }
    }
}