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
use crate::MMTK;
use crate::util::*;
use crate::vm::VMBinding;

use std::any::Any;
use std::sync::OnceLock;

struct MMTKSingleton {
    cell: OnceLock<Box<dyn Any + Send + Sync>>,
}

impl MMTKSingleton {
    pub const fn new() -> Self {
        Self {
            cell: OnceLock::new(),
        }
    }

    pub fn set<VM: VMBinding>(&'static self, mmtk: Box<MMTK<VM>>) {
        self.cell.set(mmtk).expect("MMTK singleton is already set");
    }

    pub fn get<VM: VMBinding>(&'static self) -> &'static MMTK<VM> {
        let mmtk = self.cell.get().expect("MMTK singleton is not set")
            as &'static Box<dyn Any + Send + Sync>;
        mmtk.as_ref().downcast_ref::<MMTK<VM>>().unwrap_or_else(|| {
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
    MMTK_SINGLETON.get()
}

/// Set the singleton MMTK instance for the mock VM. This method should only be called once.
pub fn set_singleton<VM: VMBinding>(mmtk: Box<MMTK<VM>>) {
    MMTK_SINGLETON.set(mmtk);
}

/// Bind a mutator thread to the MMTK singleton instance for MockVM.
/// For a custom mock VM type, use [`MockVM::bind_mutator`], e.g. `CustomVM::bind_mutator()`.
pub fn bind_mutator() -> VMMutatorThread {
    MockVM::<DefaultMockVMConfig>::bind_mutator()
}
