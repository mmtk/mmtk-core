// NOTE: MMTk will panic if MMTK is initialized more than once per process (this is a bug and we should fix it).
// To work around the problem, we run each of the following modules in a separate test process
// if the test initializes an MMTk intance.

// All the tests with prefix 'mock_test_' and with the feature 'mock_test' will use MockVM, and will initialize MMTk.
// To avoid re-initialization, one can have only one #[test] per module,
// or use fixtures in `crate::util::test_util::fixtures` to create one MMTk instance
// per module and reuse the instance in multiple tests.

// Mock tests can be placed anywhere in the source directory `src` or the test directory `tests`.
// * They need to be conditional compiled when the feature `mock_test` is enabled. Otherwise they cannot access `MockVM`.
// * They should have the prefix 'mock_test_' in their file name so they will be picked up by the CI testing scripts.

// Common includes for mock tests.
pub(crate) mod mock_test_prelude {
    pub use crate::memory_manager;
    pub use crate::util::test_util::fixtures::*;
    pub use crate::util::test_util::mock_method::*;
    pub use crate::util::test_util::mock_vm::*;
    pub use crate::vm::*;
}

mod mock_test_allocate_align_offset;
mod mock_test_allocate_with_disable_collection;
mod mock_test_allocate_with_initialize_collection;
mod mock_test_allocate_with_re_enable_collection;
mod mock_test_allocate_without_initialize_collection;
mod mock_test_allocator_info;
mod mock_test_barrier_slow_path_assertion;
#[cfg(feature = "is_mmtk_object")]
mod mock_test_conservatism;
mod mock_test_edges;
#[cfg(target_os = "linux")]
mod mock_test_handle_mmap_conflict;
mod mock_test_handle_mmap_oom;
mod mock_test_is_in_mmtk_spaces;
mod mock_test_issue139_allocate_non_multiple_of_min_alignment;
mod mock_test_issue867_allocate_unrealistically_large_object;
#[cfg(feature = "malloc_counted_size")]
mod mock_test_malloc_counted;
mod mock_test_malloc_ms;
#[cfg(all(target_pointer_width = "64", feature = "vm_space"))]
mod mock_test_mmtk_julia_pr_143;
#[cfg(feature = "nogc_lock_free")]
mod mock_test_nogc_lock_free;
#[cfg(target_pointer_width = "64")]
mod mock_test_vm_layout_compressed_pointer;
mod mock_test_vm_layout_default;
mod mock_test_vm_layout_heap_start;
mod mock_test_vm_layout_log_address_space;

mod mock_test_doc_avoid_resolving_allocator;
mod mock_test_doc_mutator_storage;
