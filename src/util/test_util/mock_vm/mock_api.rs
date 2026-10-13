//! This module includes the MMTK singleton for MockVM, and some wrapped APIs that interact with MockVM.
//! When this module provides a wrapped API, mock tests should use the wrapped API instead of
//! the APIs from [`crate:memory_manager`]. For example, [`bind_mutator`] is provided here as a wrapped API
//! which not only calls [`crate::memory_manager::bind_mutator`], but also registers the returned mutator
//! to MockVM.
//!
//! The singleton can be an MMTK instance of any mock VM type (see [`crate::define_mock_vm`]). It is
//! type-erased, and is checked against the expected type when it is accessed.

use super::DefaultMockVMConfig;
use super::MockVM;
use crate::util::*;
use crate::vm::VMBinding;
use crate::MMTK;

use std::any::Any;
use std::ptr::NonNull;
use std::sync::OnceLock;

struct MMTKSingleton {
    cell: OnceLock<NonNull<dyn Any>>,
}
unsafe impl Sync for MMTKSingleton {}

impl MMTKSingleton {
    pub const fn new() -> Self {
        Self {
            cell: OnceLock::new(),
        }
    }

    pub fn set<VM: VMBinding>(&'static self, mmtk_ptr: *mut MMTK<VM>) {
        let non_null = NonNull::new(mmtk_ptr as *mut dyn Any).unwrap();
        self.cell
            .set(non_null)
            .expect("MMTK singleton is already set");
    }

    pub fn get_mut<VM: VMBinding>(&'static self) -> &'static mut MMTK<VM> {
        let ptr = self.cell.get().expect("MMTK singleton is not set");
        unsafe { &mut *ptr.as_ptr() }
            .downcast_mut::<MMTK<VM>>()
            .unwrap_or_else(|| {
                panic!(
                    "MMTK singleton is not a {}",
                    std::any::type_name::<MMTK<VM>>()
                )
            })
    }
}

/// A singleton MMTK instance for the mock VM.
static MMTK_SINGLETON: MMTKSingleton = MMTKSingleton::new();

/// Get the singleton MMTK instance for the mock VM type `VM`.
pub fn singleton<VM: VMBinding>() -> &'static MMTK<VM> {
    singleton_mut()
}

/// Get a mutable reference to the singleton MMTK instance for the mock VM type `VM`.
pub fn singleton_mut<VM: VMBinding>() -> &'static mut MMTK<VM> {
    MMTK_SINGLETON.get_mut()
}

/// Set the singleton MMTK instance for the mock VM. This method should only be called once.
pub fn set_singleton<VM: VMBinding>(mmtk_ptr: *mut MMTK<VM>) {
    MMTK_SINGLETON.set(mmtk_ptr);
}

/// Bind a mutator thread to the MMTK singleton instance for MockVM.
/// For a custom mock VM type, use [`MockVM::bind_mutator`], e.g. `CustomVM::bind_mutator()`.
pub fn bind_mutator() -> VMMutatorThread {
    MockVM::<DefaultMockVMConfig>::bind_mutator()
}
