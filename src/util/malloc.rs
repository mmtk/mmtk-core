pub use jemalloc_sys::{calloc as je_calloc, free as je_free, malloc_usable_size as je_malloc_usable_size};
pub use mimalloc_sys::{mi_calloc, mi_free, mi_malloc_usable_size};
pub use hoard_sys::{calloc as ho_calloc, free as ho_free, malloc_usable_size as ho_malloc_usable_size};
// pub use scalloc_sys::{calloc as sc_calloc, free as sc_free, malloc_usable_size as sc_malloc_usable_size};
pub use libc::{calloc as c_calloc, free as c_free, malloc_usable_size as c_malloc_usable_size};

// #[cfg(feature="ms_jemalloc")]
// pub use jemalloc_sys::{calloc as ms_calloc, free as ms_free, malloc_usable_size as ms_malloc_usable_size};




// // Don't use TCMalloc, it doesn't work
// #[cfg(feature = "malloc_tcmalloc")]
// pub use tcmalloc_sys::{
//     TCMallocInternalCalloc as calloc,
//     TCMallocInternalFree as free,
//     TCMallocInternalMallocSize as malloc_usable_size,
// };


// export LD_LIBRARY_PATH=./../../../scalloc-sys/scalloc/out/Release/lib.target

// #[cfg(not(any(
//     feature = "malloc_jemalloc",
//     feature = "malloc_mimalloc",
//     feature = "malloc_tcmalloc",
//     feature = "malloc_hoard",
//     feature = "malloc_scalloc"
// )))]
