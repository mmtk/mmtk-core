use libc::{size_t, c_void};

#[link(name = "tcmalloc", kind = "static")]
extern "C" {
    pub fn TCMallocInternalMalloc(n: size_t, size: size_t) -> *mut c_void;
}