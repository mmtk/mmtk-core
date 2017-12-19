use ::util::Address;

use super::jtoc::*;
use super::JTOC_BASE;

const BOOT_THREAD: usize = 1;

// FIXME: This macro does not work reliably work unless wrapped
//        in a function with an `#[inline(never)]` pragma
#[cfg(target_arch = "x86")]
macro_rules! jtoc_call {
    ($offset:ident, $thread_id:expr $(, $arg:ident)*) => (unsafe {
        let ret: usize;
        let call_addr = (JTOC_BASE + $offset).load::<fn()>();
        let rvm_thread
        = Address::from_usize(((JTOC_BASE + THREAD_BY_SLOT_FIELD_JTOC_OFFSET)
            .load::<usize>() + 4 * $thread_id)).load::<usize>();

        jtoc_args!($($arg),*);

        asm!("mov esi, ebx" : : "{ebx}"(rvm_thread) : "esi" : "intel");
        asm!("call ebx" : : "{ebx}"(call_addr) : "eax" : "intel");

        asm!("mov $0, eax" : "=r"(ret) : : : "intel");
        ret
    });
}

#[cfg(target_arch = "x86")]
macro_rules! jtoc_args {
    () => ();

    ($arg1:ident) => (
        asm!("push eax" : : "{eax}"($arg1) : "memory" : "intel");
    );

    ($arg1:ident, $arg2:ident) => (
        jtoc_args!($arg1);
        asm!("push edx" : : "{edx}"($arg2) : "memory" : "intel");
    );

    ($arg1:ident, $arg2:ident, $($arg:ident),+) => (
        jtoc_args!($arg1, $arg2);
        $(
            asm!("push ebx" : : "{ebx}"($arg) : "memory" : "intel");
        )*
    );
}

#[inline(never)]
pub fn test(input: usize) -> usize {
    jtoc_call!(TEST_METHOD_JTOC_OFFSET, BOOT_THREAD, input)
}

#[inline(never)]
pub fn test1() -> usize {
    jtoc_call!(TEST1_METHOD_JTOC_OFFSET, BOOT_THREAD)
}

#[inline(never)]
pub fn test2(input1: usize, input2: usize) -> usize {
    jtoc_call!(TEST2_METHOD_JTOC_OFFSET, BOOT_THREAD, input1, input2)
}

#[inline(never)]
pub fn test3(input1: usize, input2: usize, input3: usize, input4: usize) -> usize {
    jtoc_call!(TEST3_METHOD_JTOC_OFFSET, BOOT_THREAD, input1, input2, input3, input4)
}

#[inline(never)]
pub fn stop_all_mutators(thread_id: usize) {
    jtoc_call!(BLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
}

#[inline(never)]
pub fn resume_mutators(thread_id: usize) {
    jtoc_call!(UNBLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
}

#[inline(never)]
pub fn block_for_gc(thread_id: usize) {
    jtoc_call!(BLOCK_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
}