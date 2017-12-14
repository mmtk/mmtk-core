#[cfg(feature = "jikesrvm")]
use ::util::Address;

#[cfg(feature = "jikesrvm")]
use ::vm::jtoc::*;

#[cfg(feature = "jikesrvm")]
use ::vm::JTOC_BASE;

#[cfg(feature = "jikesrvm")]
pub fn stop_all_mutators() {
    unsafe {
        (JTOC_BASE + BLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET).load::<fn()>()();
    }
}

#[cfg(not(feature = "jikesrvm"))]
pub fn stop_all_mutators() {
    unimplemented!()
}

#[cfg(feature = "jikesrvm")]
pub fn resume_mutators() {
    unsafe {
        (JTOC_BASE + UNBLOCK_ALL_MUTATORS_FOR_GC_METHOD_JTOC_OFFSET).load::<fn()>()();
    }
}

#[cfg(not(feature = "jikesrvm"))]
pub fn resume_mutators() {
    unimplemented!()
}

#[cfg(feature = "jikesrvm")]
#[cfg(target_arch = "x86")]
pub fn block_for_gc(thread_id: usize) {
    unsafe {
        let call_addr = (JTOC_BASE + BLOCK_FOR_GC_METHOD_JTOC_OFFSET).load::<fn()>();
        let rvm_thread
        = Address::from_usize(((JTOC_BASE + THREAD_BY_SLOT_FIELD_JTOC_OFFSET)
            .load::<usize>() + 4 * thread_id)).load::<usize>();

        asm!("mov esi, ecx" : : "{ecx}"(rvm_thread) : "esi" : "intel");
        asm!("call ebx" : : "{ebx}"(call_addr) : "eax" : "intel");
    }
}

#[cfg(not(feature = "jikesrvm"))]
pub fn block_for_gc() {
    unimplemented!()
}