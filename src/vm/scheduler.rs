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
        //asm!("mov eax, $0" : : "0"(thread_id) : "eax" : "intel");
        (JTOC_BASE + BLOCK_FOR_GC_METHOD_JTOC_OFFSET).load::<fn()>()();
    }
}

#[cfg(not(feature = "jikesrvm"))]
pub fn block_for_gc() {
    unimplemented!()
}