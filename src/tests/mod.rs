mod mock_test_prelude {
    pub use crate::util::test_util::fixtures::*;
    pub use crate::util::test_util::mock_vm::*;
    pub use crate::memory_manager;
    pub use crate::vm::*;
}

#[cfg(feature = "mock_test")]
mod mock_test_allocate_align_offset;
#[cfg(feature = "mock_test")]
mod mock_test_allocate_with_disable_collection;
#[cfg(feature = "mock_test")]
mod mock_test_allocate_with_initialize_collection;
#[cfg(feature = "mock_test")]
mod mock_test_allocate_with_re_enable_collection;
#[cfg(feature = "mock_test")]
mod mock_test_allocate_without_initialize_collection;
#[cfg(feature = "mock_test")]
mod mock_test_allocator_info;
#[cfg(feature = "mock_test")]
mod mock_test_barrier_slow_path_assertion;
#[cfg(all(feature = "mock_test", feature = "is_mmtk_object"))]
mod mock_test_conservatism;
#[cfg(feature = "mock_test")]
mod mock_test_edges;
#[cfg(all(feature = "mock_test", target_os = "linux"))]
mod mock_test_handle_mmap_conflict;
#[cfg(feature = "mock_test")]
mod mock_test_handle_mmap_oom;
#[cfg(feature = "mock_test")]
mod mock_test_is_in_mmtk_spaces;
#[cfg(feature = "mock_test")]
mod mock_test_issue139_allocate_non_multiple_of_min_alignment;
#[cfg(feature = "mock_test")]
mod mock_test_issue867_allocate_unrealistically_large_object;

#[cfg(feature = "mock_test")]
mod mock_test_doc_avoid_resolving_allocator;
#[cfg(feature = "mock_test")]
mod mock_test_doc_mutator_storage;
