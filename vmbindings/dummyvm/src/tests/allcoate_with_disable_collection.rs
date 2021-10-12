use crate::api::*;
use mmtk::util::opaque_pointer::*;
use mmtk::AllocationSemantics;

/// This test allocates after calling disable_collection(). When we exceed the heap limit, MMTk will NOT trigger a GC.
/// And the allocation will succeed.
#[test]
pub fn allocate_with_disable_collection() {
    const MB: usize = 1024 * 1024;
    // 1MB heap
    gc_init(MB);
    enable_collection(VMThread::UNINITIALIZED);
    let handle = bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));
    // Allocate 1MB. It should be fine.
    let addr = alloc(handle, MB, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
    // Disable GC
    disable_collection();
    // Allocate another MB. This exceeds the heap size. But as we have disabled GC, MMTk will not trigger a GC, and allow this allocation.
    let addr = alloc(handle, MB, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
}