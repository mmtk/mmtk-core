// Export one of the malloc libraries.

cfg_if::cfg_if! {
    // These two libraries support all the platforms.
    if #[cfg(feature = "malloc_jemalloc")] {
        pub use self::jemalloc::*;
    } else if #[cfg(feature = "malloc_mimalloc")] {
        pub use self::mimalloc::*;
    } else if #[cfg(target_os = "windows")] {
        // Use our own Windows malloc implementation on Windows.
        pub use self::win_malloc::*;
    } else {
        // Otherwise use libc's implementation.
        pub use self::libc_malloc::*;
    }
}

/// When we count page usage of library malloc, we assume they allocate in pages. For some malloc implementations,
/// they may use a larger page (e.g. mimalloc's 64K page). For libraries that we are not sure, we assume they use
/// normal 4k pages.
pub const BYTES_IN_MALLOC_PAGE: usize = 1 << LOG_BYTES_IN_MALLOC_PAGE;

// Different malloc libraries

// TODO: We should conditinally include some methods in the module, such as posix extension and GNU extension.

#[cfg(feature = "malloc_jemalloc")]
mod jemalloc {
    // Normal 4K page
    pub const LOG_BYTES_IN_MALLOC_PAGE: u8 = crate::util::constants::LOG_BYTES_IN_PAGE;
    // Whether this allocator supports arbitrary-alignment allocation.
    // If true, the allocator must provide a function equivalent to
    // `posix_memalign`, and the returned pointer must be
    // fully compatible with the rest of the allocator API (i.e., it can be
    // passed to `free`, `realloc`, and `malloc_usable_size`).
    pub const SUPPORT_ALIGNED_MALLOC: bool = true;
    // ANSI C
    pub use jemalloc_sys::{calloc, free, malloc, realloc};
    // Posix
    pub use jemalloc_sys::posix_memalign;
    // GNU
    pub use jemalloc_sys::malloc_usable_size;
}

#[cfg(feature = "malloc_mimalloc")]
mod mimalloc {
    // Normal 4K page accounting
    pub const LOG_BYTES_IN_MALLOC_PAGE: u8 = crate::util::constants::LOG_BYTES_IN_PAGE;
    // Whether this allocator supports arbitrary-alignment allocation.
    // If true, the allocator must provide a function equivalent to
    // `posix_memalign`, and the returned pointer must be
    // fully compatible with the rest of the allocator API (i.e., it can be
    // passed to `free`, `realloc`, and `malloc_usable_size`).
    pub const SUPPORT_ALIGNED_MALLOC: bool = true;
    // ANSI C
    pub use mimalloc_sys::{
        mi_calloc as calloc, mi_free as free, mi_malloc as malloc, mi_realloc as realloc,
    };
    // Posix
    pub use mimalloc_sys::mi_posix_memalign as posix_memalign;
    // GNU
    pub use mimalloc_sys::mi_malloc_usable_size as malloc_usable_size;
}

/// If no malloc lib is specified, use the libc implementation
#[cfg(all(
    not(target_os = "windows"),
    not(any(feature = "malloc_jemalloc", feature = "malloc_mimalloc"))
))]
mod libc_malloc {
    // Normal 4K page
    pub const LOG_BYTES_IN_MALLOC_PAGE: u8 = crate::util::constants::LOG_BYTES_IN_PAGE;
    // Whether this allocator supports arbitrary-alignment allocation.
    // If true, the allocator must provide a function equivalent to
    // `posix_memalign`, and the returned pointer must be
    // fully compatible with the rest of the allocator API (i.e., it can be
    // passed to `free`, `realloc`, and `malloc_usable_size`).
    pub const SUPPORT_ALIGNED_MALLOC: bool = true;
    // ANSI C
    pub use libc::{calloc, free, malloc, realloc};
    // Posix
    pub use libc::posix_memalign;
    // GNU
    #[cfg(target_os = "linux")]
    pub use libc::malloc_usable_size;
    #[cfg(target_os = "macos")]
    extern "C" {
        pub fn malloc_size(ptr: *const libc::c_void) -> usize;
    }
    #[cfg(target_os = "macos")]
    pub use self::malloc_size as malloc_usable_size;
}

/// Windows malloc implementation using HeapAlloc
#[cfg(all(
    target_os = "windows",
    not(any(feature = "malloc_jemalloc", feature = "malloc_mimalloc"))
))]
mod win_malloc {
    // Normal 4K page
    pub const LOG_BYTES_IN_MALLOC_PAGE: u8 = crate::util::constants::LOG_BYTES_IN_PAGE;
    // Whether this allocator supports arbitrary-alignment allocation.
    // If true, the allocator must provide a function equivalent to
    // `posix_memalign`, and the returned pointer must be
    // fully compatible with the rest of the allocator API (i.e., it can be
    // passed to `free`, `realloc`, and `malloc_usable_size`).
    // This is false for Windows. Windows provides _aligned_malloc, however, the returned value from _aligned_malloc is not compatible with other APIs.
    pub const SUPPORT_ALIGNED_MALLOC: bool = false;

    use std::ffi::c_void;
    use windows_sys::Win32::System::Memory::*;

    pub unsafe fn malloc(size: usize) -> *mut c_void {
        HeapAlloc(GetProcessHeap(), 0, size)
    }

    pub unsafe fn free(ptr: *mut c_void) {
        if !ptr.is_null() {
            HeapFree(GetProcessHeap(), 0, ptr);
        }
    }

    pub unsafe fn calloc(nmemb: usize, size: usize) -> *mut c_void {
        let total = nmemb * size;
        HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, total)
    }

    pub unsafe fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
        if ptr.is_null() {
            return malloc(size);
        }
        HeapReAlloc(GetProcessHeap(), 0, ptr, size)
    }

    pub unsafe fn malloc_usable_size(ptr: *const c_void) -> usize {
        HeapSize(GetProcessHeap(), 0, ptr)
    }

    // On Windows, there is no equivalent function to posix_memalign.
    pub unsafe fn posix_memalign(_ptr: *mut *mut c_void, _align: usize, _size: usize) -> i32 {
        // We should never call this. We state SUPPORT_ALIGNED_MALLOC as false, so the code that calls posix_memalign should never be called.
        unreachable!()
    }
}
