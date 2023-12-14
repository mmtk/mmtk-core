// NOTE: MMTk will panic if MMTK is initialized more than once per process (this is a bug and we should fix it).
// To work around the problem, we run each of the following modules in a separate test process
// if the test initializes an MMTk intance.

// All the tests with prefix 'mock_test_' and with the feature 'mock_test' will use MockVM, and will initialize MMTk.
// To avoid re-initialization, one can have only one #[test] per module,
// or use fixtures in `crate::util::test_util::fixtures` to create one MMTk instance
// per module and reuse the instance in multiple tests.

#[cfg(feature = "mock_test")]
mod mock_test_prelude {
    pub use crate::memory_manager;
    pub use crate::util::test_util::fixtures::*;
    pub use crate::util::test_util::mock_method::*;
    pub use crate::util::test_util::mock_vm::*;
    pub use crate::vm::*;
}

#[cfg(not(feature = "malloc_counted_size"))]
mod malloc_api;

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
#[cfg(all(feature = "mock_test", feature = "malloc_counted_size"))]
mod mock_test_malloc_counted;
#[cfg(feature = "mock_test")]
mod mock_test_malloc_ms;
#[cfg(all(feature = "mock_test", feature = "nogc_lock_free"))]
mod mock_test_nogc_lock_free;
#[cfg(all(feature = "mock_test", target_pointer_width = "64"))]
mod mock_test_vm_layout_compressed_pointer;
#[cfg(feature = "mock_test")]
mod mock_test_vm_layout_default;
#[cfg(feature = "mock_test")]
mod mock_test_vm_layout_heap_start;
#[cfg(feature = "mock_test")]
mod mock_test_vm_layout_log_address_space;

#[cfg(feature = "mock_test")]
mod mock_test_doc_avoid_resolving_allocator;
#[cfg(feature = "mock_test")]
mod mock_test_doc_mutator_storage;
