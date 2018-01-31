use super::super::Collection;

use ::util::Address;
use ::plan::ParallelCollector;

use super::entrypoint::*;
use super::JTOC_BASE;

pub const BOOT_THREAD: usize = 1;

pub struct VMCollection {}

impl Collection for VMCollection {
    #[inline(always)]
    fn stop_all_mutators(thread_id: usize) {
        unsafe {
            jtoc_call!(BLOCK_ALL_MUTATORS_FOR_GC_METHOD_OFFSET, thread_id);
        }
    }

    #[inline(always)]
    fn resume_mutators(thread_id: usize) {
        unsafe {
            jtoc_call!(UNBLOCK_ALL_MUTATORS_FOR_GC_METHOD_OFFSET, thread_id);
        }
    }

    #[inline(always)]
    fn block_for_gc(thread_id: usize) {
        unsafe {
            jtoc_call!(BLOCK_FOR_GC_METHOD_OFFSET, thread_id);
        }
    }

    #[inline(always)]
    unsafe fn spawn_worker_thread<T: ParallelCollector>(thread_id: usize, ctx: *mut T) {
        jtoc_call!(SPAWN_COLLECTOR_THREAD_METHOD_OFFSET, thread_id, ctx);
    }
}

impl VMCollection {
    #[inline(always)]
    pub unsafe fn thread_from_id(thread_id: usize) -> Address {
        Address::from_usize(Address::from_usize((JTOC_BASE + THREAD_BY_SLOT_FIELD_OFFSET)
            .load::<usize>() + 4 * thread_id).load::<usize>())
    }

    #[inline(always)]
    pub unsafe fn thread_from_index(thread_index: usize) -> Address {
        Address::from_usize(Address::from_usize((JTOC_BASE + THREADS_FIELD_OFFSET)
            .load::<usize>() + 4 * thread_index).load::<usize>())
    }
}