use crate::api::*;
use mmtk::util::opaque_pointer::*;
use mmtk::AllocationSemantics;

/// This test allocates without calling enable_collection(). When we exceed the heap limit, a GC should be triggered by MMTk.
/// But as we haven't enabled collection, GC is not initialized, so MMTk will panic.
#[test]
#[should_panic(expected = "GC is not allowed here")]
pub fn allocate_without_enable_collection() {
    const MB: usize = 1024 * 1024;
    // 1MB heap
    gc_init(MB);
    let handle = bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));
    // Attempt to allocate 2MB memory. This should trigger a GC, but as we never call enable_collection(), we cannot do GC.
    let addr = alloc(handle, 2 * MB, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
}