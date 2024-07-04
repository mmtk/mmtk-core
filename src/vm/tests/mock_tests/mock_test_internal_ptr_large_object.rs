// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=is_mmtk_object

use super::mock_test_prelude::*;

use crate::util::constants::LOG_BITS_IN_WORD;
use crate::util::is_mmtk_object::VO_BIT_REGION_SIZE;
use crate::util::*;

#[test]
pub fn interior_poiner_in_large_object() {
    with_mockvm(
        default_setup,
        || {
            const MB: usize = 1024 * 1024;
            let mut fixture = MutatorFixture::create_with_heapsize(10 * MB);

            let addr = memory_manager::alloc(&mut fixture.mutator, MB, 4096, 0, AllocationSemantics::LOS);
            assert!(!addr.is_zero());

            let obj = MockVM::addr_to_ref(addr);
        }
    )
}
