use super::mock_test_prelude::*;

use crate::util::opaque_pointer::*;
use crate::AllocationSemantics;

/// This test allocates without calling initialize_collection(). When we exceed the heap limit, a GC should be triggered by MMTk.
/// But as we haven't enabled collection, GC is not initialized, so MMTk will panic.
#[test]
#[should_panic(expected = "GC is not allowed here")]
pub fn allocate_without_initialize_collection() {
    // 1MB heap
    with_mockvm(
        default_setup,
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MMTKFixture::create_with_builder(
                |builder| {
                    builder
                        .options
                        .gc_trigger
                        .set(crate::util::options::GCTriggerSelector::FixedHeapSize(MB));
                },
                false,
            ); // Do not initialize collection

            // Build mutator
            let mut mutator = memory_manager::bind_mutator(
                fixture.mmtk,
                VMMutatorThread(VMThread::UNINITIALIZED),
            );

            // Allocate half MB. It should be fine.
            let addr =
                memory_manager::alloc(&mut mutator, MB >> 1, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());

            // Fill up the heap
            let _ =
                memory_manager::alloc(&mut mutator, MB >> 1, 8, 0, AllocationSemantics::Default);

            // Attempt another allocation.
            let addr = memory_manager::alloc(&mut mutator, MB, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());
        },
        || {
            // We panic before calling block_for_gc.
            read_mockvm(|mock| {
                assert!(!mock.block_for_gc.is_called());
            });
        },
    )
}
