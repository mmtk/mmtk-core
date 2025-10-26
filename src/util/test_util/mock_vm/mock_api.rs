use super::vm;
use super::MockVM;
use crate::util::*;
use crate::Mutator;
use crate::MMTK;

pub static mut MMTK_SINGLETON: *mut MMTK<MockVM> = std::ptr::null_mut();

pub fn singleton() -> &'static MMTK<MockVM> {
    unsafe {
        assert!(!MMTK_SINGLETON.is_null(), "MMTK singleton is not set");
        &*MMTK_SINGLETON
    }
}

pub fn singleton_mut() -> &'static mut MMTK<MockVM> {
    unsafe {
        assert!(!MMTK_SINGLETON.is_null(), "MMTK singleton is not set");
        &mut *MMTK_SINGLETON
    }
}

pub fn set_singleton(mmtk_ptr: *mut MMTK<MockVM>) {
    unsafe {
        MMTK_SINGLETON = mmtk_ptr;
    }
}

impl VMMutatorThread {
    pub fn as_mock_mutator(self) -> &'static mut Mutator<MockVM> {
        unsafe { &mut *(*self.0 .0.to_address().to_mut_ptr::<vm::MutatorHandle>()).ptr }
    }
}

pub fn bind_mutator() -> VMMutatorThread {
    let mmtk = singleton();
    let mutator_handle = Box::new(vm::MutatorHandle {
        ptr: std::ptr::null_mut(),
    });
    let mutator_handle_ptr = Box::into_raw(mutator_handle);
    let tls = VMMutatorThread(VMThread(OpaquePointer::from_address(
        Address::from_mut_ptr(mutator_handle_ptr),
    )));

    let mutator = crate::memory_manager::bind_mutator(mmtk, tls);
    let mutator_ptr = Box::into_raw(mutator);

    unsafe {
        (*mutator_handle_ptr).ptr = mutator_ptr;
    }

    vm::MUTATOR_PARK.register(tls.0);
    tls
}
