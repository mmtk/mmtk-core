use super::jtoc::*;
use super::JTOC_BASE;
use super::scheduling::VMScheduling;
use ::util::Address;
use ::plan::{TraceLocal};

static mut REF_SLOT_SIZE: usize = 0;
static mut CHUNK_SIZE_MASK: usize = 0;

pub unsafe fn set_ref_slot_size(thread_id: usize) {
    REF_SLOT_SIZE = jtoc_call!(GET_REFERENCE_SLOT_SIZE_METHOD_JTOC_OFFSET, thread_id);
    CHUNK_SIZE_MASK = 0xFFFFFFFF - (REF_SLOT_SIZE - 1);
}

pub fn scan_statics<T: TraceLocal>(trace: &mut T, thread_id: usize) {
    unsafe {
        let slots = JTOC_BASE;
        let thread = VMScheduling::thread_from_id(thread_id);
    }
}