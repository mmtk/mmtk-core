//! This module includes the MMTK singleton for MockVM, and some wrapped APIs that interact with MockVM.
//! When this module provides a wrapped API, mock tests should use the wrapped API instead of
//! the APIs from [`crate:memory_manager`]. For example, [`bind_mutator`] is provided here as a wrapped API
//! which not only calls [`crate::memory_manager::bind_mutator`], but also registers the returned mutator
//! to MockVM.

use super::vm;
use super::MockVM;
use crate::util::*;
use crate::MMTK;

/// A singleton MMTK instance for MockVM.
pub static mut MMTK_SINGLETON: *mut MMTK<MockVM> = std::ptr::null_mut();

/// Get the singleton MMTK instance for MockVM.
pub fn singleton() -> &'static MMTK<MockVM> {
    unsafe {
        assert!(!MMTK_SINGLETON.is_null(), "MMTK singleton is not set");
        &*MMTK_SINGLETON
    }
}

/// Get a mutable reference to the singleton MMTK instance for MockVM.
pub fn singleton_mut() -> &'static mut MMTK<MockVM> {
    unsafe {
        assert!(!MMTK_SINGLETON.is_null(), "MMTK singleton is not set");
        &mut *MMTK_SINGLETON
    }
}

/// Set the singleton MMTK instance for MockVM. This method should only be called once.
pub fn set_singleton(mmtk_ptr: *mut MMTK<MockVM>) {
    unsafe {
        assert!(MMTK_SINGLETON.is_null(), "MMTK singleton is already set");
        MMTK_SINGLETON = mmtk_ptr;
    }
}

/// Bind a mutator thread to the MMTK singleton instance for MockVM.
pub fn bind_mutator() -> VMMutatorThread {
    vm::MutatorHandle::bind()
}
