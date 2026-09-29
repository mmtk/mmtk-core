//! This module includes the MMTK singleton for MockVM, and some wrapped APIs that interact with MockVM.
//! When this module provides a wrapped API, mock tests should use the wrapped API instead of
//! the APIs from [`crate:memory_manager`]. For example, [`bind_mutator`] is provided here as a wrapped API
//! which not only calls [`crate::memory_manager::bind_mutator`], but also registers the returned mutator
//! to MockVM.
//!
//! The singleton can be an MMTK instance of any mock VM type (see [`crate::define_mock_vm`]). It is
//! type-erased, and is checked against the expected type when it is accessed.

use super::MockVM;
use crate::util::*;
use crate::vm::VMBinding;
use crate::MMTK;

use std::any::Any;
use std::ptr::NonNull;

/// A singleton MMTK instance for the mock VM.
static mut MMTK_SINGLETON: Option<NonNull<dyn Any>> = None;

/// Get the singleton MMTK instance for the mock VM type `VM`.
pub fn singleton<VM: VMBinding>() -> &'static MMTK<VM> {
    singleton_mut()
}

/// Get a mutable reference to the singleton MMTK instance for the mock VM type `VM`.
pub fn singleton_mut<VM: VMBinding>() -> &'static mut MMTK<VM> {
    let ptr = unsafe { MMTK_SINGLETON }.expect("MMTK singleton is not set");
    unsafe { &mut *ptr.as_ptr() }
        .downcast_mut::<MMTK<VM>>()
        .unwrap_or_else(|| {
            panic!(
                "MMTK singleton is not a {}",
                std::any::type_name::<MMTK<VM>>()
            )
        })
}

/// Set the singleton MMTK instance for the mock VM. This method should only be called once.
pub fn set_singleton<VM: VMBinding>(mmtk_ptr: *mut MMTK<VM>) {
    unsafe {
        assert!(
            (*std::ptr::addr_of!(MMTK_SINGLETON)).is_none(),
            "MMTK singleton is already set"
        );
        MMTK_SINGLETON = Some(NonNull::new(mmtk_ptr as *mut dyn Any).unwrap());
    }
}

/// Bind a mutator thread to the MMTK singleton instance for MockVM.
/// For a custom mock VM type, use [`super::GenericMockVM::bind_mutator`].
pub fn bind_mutator() -> VMMutatorThread {
    MockVM::bind_mutator()
}
