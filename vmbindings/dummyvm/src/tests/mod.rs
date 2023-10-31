// NOTE: Since the dummyvm uses a global MMTK instance,
// it will panic if MMTK initialized more than once per process.
// We run each of the following modules in a separate test process.
//
// One way to avoid re-initialization is to have only one #[test] per module.
// There are also helpers for creating fixtures in `fixture/mod.rs`.
mod allocate_align_offset;
mod allocate_with_disable_collection;
mod allocate_with_initialize_collection;
mod allocate_with_re_enable_collection;
mod allocate_without_initialize_collection;
mod allocator_info;
mod barrier_slow_path_assertion;
#[cfg(feature = "is_mmtk_object")]
mod conservatism;
mod edges_test;
#[cfg(target_os = "linux")]
mod handle_mmap_conflict;
mod handle_mmap_oom;
mod is_in_mmtk_spaces;
mod issue139_allocate_unaligned_object_size;
mod issue867_allocate_unrealistically_large_object;
#[cfg(not(feature = "malloc_counted_size"))]
mod malloc_api;
#[cfg(feature = "malloc_counted_size")]
mod malloc_counted;
mod malloc_ms;
#[cfg(feature = "nogc_lock_free")]
mod nogc_lock_free;
#[cfg(target_pointer_width = "64")]
mod vm_layout_compressed_pointer_64;
mod vm_layout_default;
mod vm_layout_heap_start;
mod vm_layout_log_address_space;

// The code snippets of these tests are also referred in our docs.
mod doc_avoid_resolving_allocator;
mod doc_mutator_storage;
