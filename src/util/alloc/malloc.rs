// Import calloc, free, and malloc_usable_size from the library specified in Cargo.toml:45
#[cfg(feature = "malloc_jemalloc")]
pub use jemalloc_sys::{calloc, free, malloc_usable_size};

#[cfg(feature = "malloc_mimalloc")]
pub use mimalloc_sys::{
    mi_calloc as calloc,
    mi_free as free,
    mi_malloc_usable_size as malloc_usable_size,
};

// Don't use TCMalloc, it doesn't work
#[cfg(feature = "malloc_tcmalloc")]
pub use tcmalloc_sys::{
    TCMallocInternalCalloc as calloc,
    TCMallocInternalFree as free,
    TCMallocInternalMallocSize as malloc_usable_size,
};

// export LD_LIBRARY_PATH=./../../mmtk/target/release/build/hoard-sys-f2b4a059118f1d26/out/Hoard/src
#[cfg(feature = "malloc_hoard")]
pub use hoard_sys::{calloc, free, malloc_usable_size};

// export LD_LIBRARY_PATH=./../../../scalloc-sys/scalloc/out/Release/lib.target
#[cfg(feature = "malloc_scalloc")]
pub use scalloc_sys::{calloc, free, malloc_usable_size};

#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_tcmalloc",
    feature = "malloc_hoard",
    feature = "malloc_scalloc"
)))]
pub use libc::{calloc, free, malloc_usable_size};