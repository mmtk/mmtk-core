use std::alloc::GlobalAlloc;

use libc::c_void;

#[cfg(feature = "ga_jemalloc")]
pub use crate::util::malloc::{je_calloc as ga_calloc, je_free as ga_free};
#[cfg(feature = "ga_mimalloc")]
pub use crate::util::malloc::{mi_calloc as ga_calloc, mi_free as ga_free};
#[cfg(feature = "ga_hoard")]
pub use crate::util::malloc::{ho_calloc as ga_calloc, ho_free as ga_free};
#[cfg(feature = "ga_libc")]
pub use crate::util::malloc::{c_calloc as ga_calloc, c_free as ga_free};
// #[cfg(feature = "ga_scalloc")]
// pub use crate::util::malloc::{sc_calloc as ga_calloc, sc_free as ga_free};

#[cfg(any(
    feature = "ga_jemalloc",
    feature = "ga_mimalloc",
    feature = "ga_tcmalloc",
    feature = "ga_hoard",
    feature = "ga_libc"
    // feature = "ga_scalloc"
))]
pub struct Malloc;
#[cfg(any(
    feature = "ga_jemalloc",
    feature = "ga_mimalloc",
    feature = "ga_tcmalloc",
    feature = "ga_hoard",
    feature = "ga_libc"
    // feature = "ga_scalloc"
))]
unsafe impl GlobalAlloc for Malloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ga_calloc(layout.align(), layout.size()) as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: std::alloc::Layout) {
        ga_free(ptr as *mut c_void)
    }
}
#[cfg(any(
    feature = "ga_jemalloc",
    feature = "ga_mimalloc",
    feature = "ga_tcmalloc",
    feature = "ga_hoard",
    feature = "ga_libc"
    // feature = "ga_scalloc"
))]
#[global_allocator]
static GLOBAL: Malloc = Malloc;