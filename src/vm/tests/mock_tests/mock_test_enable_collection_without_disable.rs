use crate::memory_manager;
use crate::util::test_util::fixtures::*;
use crate::util::test_util::mock_vm::*;

/// Calling `enable_collection()` without a prior matching call to `disable_collection()` must
/// not panic. Collection is already enabled, so it should be a no-op: it returns `false` and
/// collection remains enabled.
#[test]
pub fn enable_collection_without_disable_is_noop() {
    with_mockvm(
        MockVM::default,
        || {
            let fixture = MutatorFixture::create_with_heapsize(1024 * 1024);
            let mmtk = fixture.mmtk();

            assert!(memory_manager::is_collection_enabled(mmtk));
            assert!(!memory_manager::enable_collection(mmtk));
            assert!(memory_manager::is_collection_enabled(mmtk));
        },
        no_cleanup,
    )
}
