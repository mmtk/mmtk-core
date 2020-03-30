use crate::util::Address;
use libc::c_void;

// This is mainly used to represent TLS.
// OpaquePointer does not provide any method for dereferencing, as we should not dereference it in MMTk.
// However, there are occurrences that we may need to dereference tls in the VM binding code.
// In JikesRVM's implementation of ActivePlan, we need to dereference tls to get mutator and collector context.
// This is done by transmute (unsafe).
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OpaquePointer(*mut c_void);

unsafe impl Sync for OpaquePointer {}
unsafe impl Send for OpaquePointer {}

impl OpaquePointer {
    pub const UNINITIALIZED: Self = Self(0 as *mut c_void);

    pub fn from_address(addr: Address) -> Self {
        OpaquePointer(addr.to_mut_ptr::<c_void>())
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }
}
