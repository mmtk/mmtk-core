use crate::util::Address;
use libc::c_void;

/// OpaquePointer represents pointers that MMTk needs to know about but will not deferefence it.
/// For example, a pointer to the thread or the thread local storage is an opaque pointer for MMTK.
/// The type does not provide any method for dereferencing.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OpaquePointer(*mut c_void);

// We never really dereference an opaque pointer in mmtk-core.
unsafe impl Sync for OpaquePointer {}
unsafe impl Send for OpaquePointer {}

impl Default for OpaquePointer {
    fn default() -> Self {
        Self::UNINITIALIZED
    }
}

impl OpaquePointer {
    /// Represents an uninitialized value for [`OpaquePointer`].
    pub const UNINITIALIZED: Self = Self(0 as *mut c_void);

    /// Cast an [`Address`] type to an [`OpaquePointer`].
    pub fn from_address(addr: Address) -> Self {
        OpaquePointer(addr.to_mut_ptr::<c_void>())
    }

    /// Cast the opaque pointer to an [`Address`] type.
    pub fn to_address(self) -> Address {
        Address::from_mut_ptr(self.0)
    }

    /// Is this opaque pointer null?
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }
}

/// A VMThread is an opaque pointer that can uniquely identify a thread in the VM.
/// A VM binding may use thread pointers or thread IDs as VMThreads. MMTk does not make any assumption on this.
/// This is used as arguments in the VM->MMTk APIs, and MMTk may store it and pass it back through the MMTk->VM traits,
/// so the VM knows the context.
/// A VMThread may be a VMMutatorThread, a VMWorkerThread, or any VMThread.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct VMThread(pub OpaquePointer);

impl VMThread {
    /// Represents an uninitialized value for [`VMThread`].
    pub const UNINITIALIZED: Self = Self(OpaquePointer::UNINITIALIZED);
}

/// A VMMutatorThread is a VMThread that associates with a [`crate::plan::Mutator`].
/// When a VMMutatorThread is used as an argument or a field of a type, it generally means
/// the function or the functions for the type is executed in the context of the mutator thread.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct VMMutatorThread(pub VMThread);

/// A VMWorkerThread is a VMThread that is associates with a [`crate::scheduler::GCWorker`].
/// When a VMWorkerThread is used as an argument or a field of a type, it generally means
/// the function or the functions for the type is executed in the context of the mutator thread.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct VMWorkerThread(pub VMThread);
