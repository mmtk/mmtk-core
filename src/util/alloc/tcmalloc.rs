use libc::{size_t, c_void};

#[cfg(feature = "malloc_tcmalloc")]
#[link(name = "tcmalloc", kind = "static")]
extern "C" {
    pub fn TCMallocInternalMalloc(n: size_t, size: size_t) -> *mut c_void;
}