#[cfg(feature = "jikesrvm")]
use ::util::Address;

#[cfg(feature = "jikesrvm")]
use ::vm::jtoc::*;

#[cfg(feature = "jikesrvm")]
use ::vm::JTOC_BASE;

#[cfg(feature = "jikesrvm")]
const BOOT_THREAD: usize = 1;

#[cfg(feature = "jikesrvm")]
macro_rules! jtoc_call {
    ($offset:ident, $thread_id:expr) => (unsafe {
        let call_addr = (JTOC_BASE + $offset).load::<fn()>();
        let rvm_thread
        = Address::from_usize(((JTOC_BASE + THREAD_BY_SLOT_FIELD_JTOC_OFFSET)
            .load::<usize>() + 4 * $thread_id)).load::<usize>();

        asm!("mov esi, ebx" : : "{ebx}"(rvm_thread) : "esi" : "intel");
        //asm!("mov r14, ebx" : : "{ebx}"(JTOC_BASE.as_usize()) : "r14" : "intel");
        asm!("call ebx" : : "{ebx}"(call_addr) : "eax" : "intel");
    });
}

#[cfg(feature = "jikesrvm")]
pub fn test1() {
    jtoc_call!(TEST1_METHOD_JTOC_OFFSET, BOOT_THREAD);
}

#[cfg(feature = "jikesrvm")]
pub fn stop_all_mutators() {
    jtoc_call!(BLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, BOOT_THREAD);
}

#[cfg(not(feature = "jikesrvm"))]
pub fn stop_all_mutators() {
    unimplemented!()
}

#[cfg(feature = "jikesrvm")]
pub fn resume_mutators() {
    jtoc_call!(UNBLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET, BOOT_THREAD);
}

#[cfg(not(feature = "jikesrvm"))]
pub fn resume_mutators() {
    unimplemented!()
}

#[cfg(feature = "jikesrvm")]
#[cfg(target_arch = "x86")]
pub fn block_for_gc(thread_id: usize) {
    jtoc_call!(BLOCK_FOR_GC_METHOD_JTOC_OFFSET, thread_id);
}

#[cfg(not(feature = "jikesrvm"))]
pub fn block_for_gc() {
    unimplemented!()
}