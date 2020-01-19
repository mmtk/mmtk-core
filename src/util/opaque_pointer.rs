use libc::c_void;
use ::util::Address;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OpaquePointer(*mut c_void);

unsafe impl Sync for OpaquePointer {}
unsafe impl Send for OpaquePointer {}

impl OpaquePointer {
    pub const UNINITIALIZED: Self = Self(0 as *mut c_void);

    pub fn from_address(addr: Address) -> Self {
        OpaquePointer(addr.to_ptr_mut::<c_void>())
    }

    pub fn is_null(&self) -> bool {
        self.0 == 0 as *mut c_void
    }
}
