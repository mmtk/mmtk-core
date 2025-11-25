// Export one of the malloc libraries.

#[cfg(feature = "malloc_jemalloc")]
pub use self::jemalloc::*;
#[cfg(all(
    not(target_os = "windows"),
    not(any(feature = "malloc_jemalloc", feature = "malloc_mimalloc"))
))]
pub use self::libc_malloc::*;
#[cfg(feature = "malloc_mimalloc")]
pub use self::mimalloc::*;
#[cfg(all(
    target_os = "windows",
    not(any(feature = "malloc_jemalloc", feature = "malloc_mimalloc"))
))]
pub use self::win_malloc::*;

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
#[cfg(target_os = "windows")]
mod win_malloc {
    // Normal 4K page
    pub const LOG_BYTES_IN_MALLOC_PAGE: u8 = crate::util::constants::LOG_BYTES_IN_PAGE;

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

    pub unsafe fn posix_memalign(memptr: *mut *mut c_void, alignment: usize, size: usize) -> i32 {
        // Windows HeapAlloc usually guarantees 16-byte alignment on 64-bit systems.
        // If the requested alignment is larger than that, we cannot satisfy it using standard HeapAlloc
        // without complex wrapping (which would require a custom free).
        // For now, we return EINVAL if the alignment is too large, rather than returning misaligned memory.
        if alignment > 16 {
            return 22; // EINVAL
        }

        let ptr = malloc(size);
        if ptr.is_null() {
            return 12; // ENOMEM
        }
        // Double check alignment
        if (ptr as usize) % alignment != 0 {
            // Should not happen for alignment <= 16 on 64-bit Windows usually.
            free(ptr);
            return 22; // EINVAL
        }
        *memptr = ptr;
        0
    }

    pub unsafe fn malloc_usable_size(ptr: *const c_void) -> usize {
        HeapSize(GetProcessHeap(), 0, ptr)
    }
}
