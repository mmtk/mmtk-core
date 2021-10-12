use crate::api::*;
use mmtk::util::opaque_pointer::*;
use mmtk::AllocationSemantics;

/// This test allocates after calling enable_collection(). When we exceed the heap limit, MMTk will trigger a GC. And block_for_gc will be called.
/// We havent implemented block_for_gc so it will panic. This test is similar to allocate_with_enable_collection, except that we once disabled GC in the test.
#[test]
#[should_panic(expected = "block_for_gc is not implemented")]
pub fn allocate_with_re_enable_collection() {
    const MB: usize = 1024 * 1024;
    // 1MB heap
    gc_init(MB);
    enable_collection(VMThread::UNINITIALIZED);
    let handle = bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));
    // Allocate 1MB. It should be fine.
    let addr = alloc(handle, MB, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
    // Disable GC. So we can keep allocate without triggering a GC.
    disable_collection();
    let addr = alloc(handle, MB, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
    // Enable GC again. When we allocate, we should see a GC triggered immediately.
    enable_collection(VMThread::UNINITIALIZED);
    let addr = alloc(handle, MB, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
}