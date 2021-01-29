#[cfg(feature = "ms_hoard")]
pub use crate::util::malloc::{
    ho_calloc as ms_calloc, ho_free as ms_free, ho_malloc_usable_size as ms_malloc_usable_size,
};
#[cfg(feature = "ms_jemalloc")]
pub use crate::util::malloc::{
    je_calloc as ms_calloc, je_free as ms_free, je_malloc_usable_size as ms_malloc_usable_size,
};
#[cfg(feature = "ms_mimalloc")]
pub use crate::util::malloc::{
    mi_calloc as ms_calloc, mi_free as ms_free, mi_malloc_usable_size as ms_malloc_usable_size,
};
// #[cfg(feature = "ms_scalloc")]
// pub use crate::util::malloc::{sc_calloc as ms_calloc, sc_free as ms_free, sc_malloc_usable_size as ms_malloc_usable_size};
#[cfg(not(any(
    feature = "ms_jemalloc",
    feature = "ms_mimalloc",
    feature = "ms_tcmalloc",
    feature = "ms_hoard",
    // feature = "ms_scalloc"
)))]
pub use crate::util::malloc::{
    c_calloc as ms_calloc, c_free as ms_free, c_malloc_usable_size as ms_malloc_usable_size,
};
