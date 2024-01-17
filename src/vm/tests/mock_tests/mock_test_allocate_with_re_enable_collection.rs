use crate::memory_manager;
use crate::util::test_util::fixtures::*;
use crate::util::test_util::mock_method::*;
use crate::util::test_util::mock_vm::*;
use crate::AllocationSemantics;

// This test allocates after calling initialize_collection(). When we exceed the heap limit, MMTk will trigger a GC. And block_for_gc will be called.
// We havent implemented block_for_gc so it will panic. This test is similar to allocate_with_initialize_collection, except that we once disabled GC in the test.
#[test]
#[should_panic(expected = "block_for_gc is called")]
pub fn allocate_with_re_enable_collection() {
    // 1MB heap
    with_mockvm(
        || -> MockVM {
            MockVM {
                block_for_gc: MockMethod::new_fixed(Box::new(|_| panic!("block_for_gc is called"))),
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

            // Disable GC. So we can keep allocate without triggering a GC.
            memory_manager::disable_collection(fixture.mmtk());
            // Fill up the heap
            let _ = memory_manager::alloc(
                &mut fixture.mutator,
                MB >> 1,
                8,
                0,
                AllocationSemantics::Default,
            );

            // Enable GC again.
            memory_manager::enable_collection(fixture.mmtk());
            // Attempt another allocation. This will trigger GC.
            let addr =
                memory_manager::alloc(&mut fixture.mutator, MB, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());
        },
        || {
            // This is actually redundant, as we defined block_for_gc for this test.
            // This just demostrates that we can check if the method is called.
            read_mockvm(|mock| {
                assert!(mock.block_for_gc.is_called());
            });
        },
    )
}
