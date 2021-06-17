#[cfg(feature = "malloc_jemalloc")]
pub use jemalloc_sys::{aligned_alloc, calloc, free, malloc_usable_size};

#[cfg(feature = "malloc_mimalloc")]
pub use mimalloc_sys::{
    mi_aligned_alloc as aligned_alloc, mi_calloc as calloc, mi_free as free,
    mi_malloc_usable_size as malloc_usable_size,
};

#[cfg(feature = "malloc_hoard")]
pub use hoard_sys::{calloc, free, malloc_usable_size};
#[cfg(feature = "malloc_hoard")]
use libc::{c_void, size_t};
#[cfg(feature = "malloc_hoard")]
pub unsafe fn aligned_alloc(alignment: size_t, size: size_t) -> *mut c_void {
    // hoard does not provide any aligned alloc. Their alloc will align to 16.
    assert!(
        alignment <= 16,
        "Hoard does not support alignment {}",
        alignment
    );
    calloc(1, size)
}

#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
pub use libc::{calloc, free, malloc_usable_size, memalign as aligned_alloc};
