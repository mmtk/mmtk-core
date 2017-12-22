use ::util::Address;

use super::jtoc::*;
use super::JTOC_BASE;

const BOOT_THREAD: usize = 1;

// TODO: #1 is fixed, but we need more guarantees that this won't gratuitously break
#[cfg(target_arch = "x86")]
macro_rules! jtoc_call {
    ($offset:ident, $thread_id:expr $(, $arg:ident)*) => (unsafe {
        let ret: usize;
        let call_addr = (JTOC_BASE + $offset).load::<fn()>();
        let rvm_thread
        = Address::from_usize(((JTOC_BASE + THREAD_BY_SLOT_FIELD_JTOC_OFFSET)
            .load::<usize>() + 4 * $thread_id)).load::<usize>();

        jtoc_args!($($arg),*);

        asm!("mov esi, ecx\n\
              call ebx\n\
              mov $0, eax" : "=r"(ret) : "{ecx}"(rvm_thread), "{ebx}"(call_addr) : "eax", "ebx", "ecx", "edx", "esi", "memory" : "intel");

        ret
    });
}

#[cfg(target_arch = "x86")]
macro_rules! jtoc_args {
    () => ();

    ($arg1:ident) => (
        asm!("push eax" : : "{eax}"($arg1) : "sp", "memory" : "intel");
    );

    ($arg1:ident, $arg2:ident) => (
        jtoc_args!($arg1);
        asm!("push edx" : : "{edx}"($arg2) : "sp", "memory" : "intel");
    );

    ($arg1:ident, $arg2:ident, $($arg:ident),+) => (
        jtoc_args!($arg1, $arg2);
        $(
            asm!("push ebx" : : "{ebx}"($arg) : "sp", "memory" : "intel");
        )*
    );
}

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