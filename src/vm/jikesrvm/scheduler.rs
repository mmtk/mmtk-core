use ::util::Address;

use super::jtoc::*;
use super::JTOC_BASE;

const BOOT_THREAD: usize = 1;

#[inline(always)]
pub fn test(input: usize) -> usize {
    jtoc_call!(TEST_METHOD_JTOC_OFFSET, BOOT_THREAD, input)
}

#[inline(always)]
pub fn test1() -> usize {
    jtoc_call!(TEST1_METHOD_JTOC_OFFSET, BOOT_THREAD)
}

#[inline(always)]
pub fn test2(input1: usize, input2: usize) -> usize {
    jtoc_call!(TEST2_METHOD_JTOC_OFFSET, BOOT_THREAD, input1, input2)
}

#[inline(always)]
pub fn test3(input1: usize, input2: usize, input3: usize, input4: usize) -> usize {
    jtoc_call!(TEST3_METHOD_JTOC_OFFSET, BOOT_THREAD, input1, input2, input3, input4)
}

#[inline(always)]
pub fn stop_all_mutators(thread_id: usize) {
    jtoc_call!(BLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
}

#[inline(always)]
pub fn resume_mutators(thread_id: usize) {
    jtoc_call!(UNBLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
}

#[inline(always)]
pub fn block_for_gc(thread_id: usize) {
    jtoc_call!(BLOCK_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
}