use crate::memory_manager;
use crate::util::test_util::fixtures::*;
use crate::util::test_util::mock_method::*;
use crate::util::test_util::mock_vm::*;
use crate::AllocationSemantics;

/// This test allocates after calling `initialize_collection()`. When we exceed the heap limit for the first time, MMTk will not trigger GC since GC has been disabled
/// However, the second 1MB allocation will trigger a GC since GC is enabled again. And `block_for_gc` will be called.
/// We haven't implemented `block_for_gc` so it will panic. This test is similar to `allocate_with_initialize_collection`, except that GC is disabled once in the test.
#[test]
#[should_panic(expected = "block_for_gc is called")]
pub fn allocate_with_re_enable_collection() {
    // 1MB heap
    with_mockvm(
        || -> MockVM {
            MockVM {
                block_for_gc: MockMethod::new_fixed(Box::new(|_| panic!("block_for_gc is called"))),
                is_collection_enabled: MockMethod::new_sequence(vec![
                    Box::new(|()| -> bool { true }), // gc is enabled but it shouldn't matter here
                    Box::new(|()| -> bool { false }), // gc is disabled
                    Box::new(|()| -> bool { true }), // gc is enabled again
                ]),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            let mut fixture = MutatorFixture::create_with_heapsize(MB);

            // Allocate half MB. It should be fine.
            let addr = memory_manager::alloc(
                &mut fixture.mutator,
                MB >> 1,
                8,
                0,
                AllocationSemantics::Default,
            );
            assert!(!addr.is_zero());

            // In the next allocation GC is disabled. So we can keep allocate without triggering a GC.
            // Fill up the heap
            let _ =
                memory_manager::alloc(&mut fixture.mutator, MB, 8, 0, AllocationSemantics::Default);

            // Attempt another allocation. This will trigger GC since GC is enabled again.
            let addr =
                memory_manager::alloc(&mut fixture.mutator, MB, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());
        },
        || {
            // This ensures that block_for_gc is called for this test, and that the second allocation
            // does not trigger GC since we expect is_collection_enabled to be called three times.
            read_mockvm(|mock| {
                assert!(mock.block_for_gc.is_called());
                assert!(mock.is_collection_enabled.call_count() == 3);
            });
        },
    )
}
