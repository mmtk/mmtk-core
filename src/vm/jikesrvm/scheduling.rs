use super::super::Scheduling;

use ::util::Address;
use ::plan::ParallelCollector;

use super::jtoc::*;
use super::JTOC_BASE;

pub const BOOT_THREAD: usize = 1;

pub struct VMScheduling {}

impl Scheduling for VMScheduling {
    #[inline(always)]
    fn stop_all_mutators(thread_id: usize) {
        jtoc_call!(BLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
    }

    #[inline(always)]
    fn resume_mutators(thread_id: usize) {
        jtoc_call!(UNBLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
    }

    #[inline(always)]
    fn block_for_gc(thread_id: usize) {
        jtoc_call!(BLOCK_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
    }

    #[inline(always)]
    fn spawn_worker_thread<T: ParallelCollector>(thread_id: usize, ctx: *mut T) {
        jtoc_call!(SPAWN_COLLECTOR_THREAD_METHOD_JTOC_OFFSET, thread_id, ctx);
    }
}

impl VMScheduling {
    #[inline(always)]
    pub unsafe fn thread_from_id(thread_id: usize) -> Address {
        Address::from_usize(Address::from_usize((JTOC_BASE + THREADS_FIELD_JTOC_OFFSET)
            .load::< usize > ()
                + 4 * thread_id).load::< usize > ())
    }
}