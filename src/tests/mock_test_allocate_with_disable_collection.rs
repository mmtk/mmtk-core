use crate::memory_manager;
use crate::util::test_util::fixtures::*;
use crate::util::test_util::mock_vm::*;
use crate::AllocationSemantics;

/// This test allocates after calling disable_collection(). When we exceed the heap limit, MMTk will NOT trigger a GC.
/// And the allocation will succeed.
#[test]
pub fn allocate_with_disable_collection() {
    with_mockvm(
        default_setup,
        || {
            // 1MB heap
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

            // Disable GC
            memory_manager::disable_collection(fixture.mmtk());
            // Allocate another MB. This exceeds the heap size. But as we have disabled GC, MMTk will not trigger a GC, and allow this allocation.
            let addr =
                memory_manager::alloc(&mut fixture.mutator, MB, 8, 0, AllocationSemantics::Default);
            assert!(!addr.is_zero());
        },
        no_cleanup,
    )
}
