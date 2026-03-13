// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=vo_bit

use super::mock_test_prelude::*;

use crate::AllocationSemantics;

#[test]
pub fn interior_pointer_before_object_ref() {
    const MB: usize = 1024 * 1024;
    const OBJECT_SIZE: usize = 16;
    with_mockvm(
        || -> MockVM {
            MockVM {
                get_object_size: MockMethod::new_fixed(Box::new(|_| OBJECT_SIZE)),
                ..MockVM::default()
            }
        },
        || {
            let mut fixture = MutatorFixture::create_with_heapsize(10 * MB);

            let addr = memory_manager::alloc(
                &mut fixture.mutator,
                OBJECT_SIZE,
                8,
                0,
                AllocationSemantics::Default,
            );
            assert!(!addr.is_zero());

            let obj = MockVM::object_start_to_ref(addr);
            println!(
                "start = {}, end = {}, obj = {}",
                addr,
                addr + OBJECT_SIZE,
                obj,
            );
            memory_manager::post_alloc(
                &mut fixture.mutator,
                obj,
                OBJECT_SIZE,
                AllocationSemantics::Default,
            );
        },
        no_cleanup,
    )
}
