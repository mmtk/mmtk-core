#[cfg(feature = "malloc_jemalloc")]
pub use jemalloc_sys::{calloc, free, malloc_usable_size};

#[cfg(feature = "malloc_mimalloc")]
pub use mimalloc_sys::{
    mi_calloc as calloc, mi_free as free, mi_malloc_usable_size as malloc_usable_size,
};

#[cfg(feature = "malloc_hoard")]
pub use hoard_sys::{calloc, free, malloc_usable_size};

#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
pub use libc::{calloc, free, malloc_usable_size};
