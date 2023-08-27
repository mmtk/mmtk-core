use crate::api::*;
use mmtk::util::opaque_pointer::*;
use mmtk::AllocationSemantics;

/// This test allocates after calling initialize_collection(). When we exceed the heap limit, MMTk will trigger a GC. And block_for_gc will be called.
/// We havent implemented block_for_gc so it will panic.
#[test]
#[should_panic(expected = "block_for_gc is not implemented")]
pub fn allocate_with_initialize_collection() {
    const MB: usize = 1024 * 1024;
    // 1MB heap
    mmtk_init(MB);
    mmtk_initialize_collection(VMThread::UNINITIALIZED);
    let handle = mmtk_bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));
    // Fill up the heap
    let _ = mmtk_alloc(handle, MB, 8, 0, AllocationSemantics::Default);
    // Attempt another allocation. This will trigger GC.
    let addr = mmtk_alloc(handle, MB, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
}
