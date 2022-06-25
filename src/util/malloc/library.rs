// Export one of the malloc libraries.

#[cfg(feature = "malloc_hoard")]
pub use self::hoard::*;
#[cfg(feature = "malloc_jemalloc")]
pub use self::jemalloc::*;
#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
pub use self::libc_malloc::*;
#[cfg(feature = "malloc_mimalloc")]
pub use self::mimalloc::*;

// Different malloc libraries

// TODO: We should conditinally include some methods in the module, such as posix extension and GNU extension.

#[cfg(feature = "malloc_jemalloc")]
mod jemalloc {
    // ANSI C
    pub use jemalloc_sys::{calloc, free, malloc, realloc};
    // Posix
    pub use jemalloc_sys::posix_memalign;
    // GNU
    pub use jemalloc_sys::malloc_usable_size;
}

#[cfg(feature = "malloc_mimalloc")]
mod mimalloc {
    // ANSI C
    pub use mimalloc_sys::{
        mi_calloc as calloc, mi_free as free, mi_malloc as malloc, mi_realloc as realloc,
    };
    // Posix
    pub use mimalloc_sys::mi_posix_memalign as posix_memalign;
    // GNU
    pub use mimalloc_sys::mi_malloc_usable_size as malloc_usable_size;
}

#[cfg(feature = "malloc_hoard")]
mod hoard {
    // ANSI C
    pub use hoard_sys::{calloc, free, malloc, realloc};
    // Posix
    pub use hoard_sys::posix_memalign;
    // GNU
    pub use hoard_sys::malloc_usable_size;
}

/// If no malloc lib is specified, use the libc implementation
#[cfg(not(any(
    feature = "malloc_jemalloc",
    feature = "malloc_mimalloc",
    feature = "malloc_hoard",
)))]
mod libc_malloc {
    // ANSI C
    pub use libc::{calloc, free, malloc, realloc};
    // Posix
    pub use libc::posix_memalign;
    // GNU
    pub use libc::malloc_usable_size;
}
