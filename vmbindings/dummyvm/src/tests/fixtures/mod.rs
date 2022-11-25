// Some tests are conditionally compiled. So not all the code in this module will be used. We simply allow dead code in this module.
#![allow(dead_code)]

use atomic_refcell::AtomicRefCell;
use std::sync::Once;
use std::sync::Mutex;

use mmtk::AllocationSemantics;
use mmtk::MMTK;
use mmtk::util::{ObjectReference, VMThread, VMMutatorThread};

use crate::api::*;
use crate::object_model::OBJECT_REF_OFFSET;
use crate::DummyVM;

pub trait FixtureContent {
    fn create() -> Self;
}


pub struct Fixture<T: FixtureContent> {
    content: AtomicRefCell<Option<Box<T>>>,
    once: Once,
}

unsafe impl<T: FixtureContent> Sync for Fixture<T> {}

impl<T: FixtureContent> Fixture<T> {
    pub fn new() -> Self {
        Self {
            content: AtomicRefCell::new(None),
            once: Once::new(),
        }
    }

    pub fn with_fixture<F: Fn(&T)>(&self, func: F) {
        self.once.call_once(|| {
            let content = Box::new(T::create());
            let mut borrow = self.content.borrow_mut();
            *borrow = Some(content);
        });
        let borrow = self.content.borrow();
        func(borrow.as_ref().unwrap())
    }
}

/// SerialFixture ensures all `with_fixture()` calls will be executed serially.
pub struct SerialFixture<T: FixtureContent> {
    content: Mutex<Option<Box<T>>>
}

impl<T: FixtureContent> SerialFixture<T> {
    pub fn new() -> Self {
        Self {
            content: Mutex::new(None)
        }
    }

    pub fn with_fixture<F: Fn(&T)>(&self, func: F) {
        let mut c = self.content.lock().unwrap();
        if c.is_none() {
            *c = Some(Box::new(T::create()));
        }
        func(c.as_ref().unwrap())
    }
}

pub struct SingleObject {
    pub objref: ObjectReference,
}

impl FixtureContent for SingleObject {
    fn create() -> Self {
        const MB: usize = 1024 * 1024;
        // 1MB heap
        mmtk_init(MB);
        mmtk_initialize_collection(VMThread::UNINITIALIZED);
        // Make sure GC does not run during test.
        mmtk_disable_collection();
        let handle = mmtk_bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));

        // A relatively small object, typical for Ruby.
        let size = 40;
        let semantics = AllocationSemantics::Default;

        let addr = mmtk_alloc(handle, size, 8, 0, semantics);
        assert!(!addr.is_zero());

        let objref = ObjectReference::from_raw_address(addr.add(OBJECT_REF_OFFSET));
        mmtk_post_alloc(handle, objref, size, semantics);

        SingleObject { objref }
    }
}

pub struct MMTKSingleton {
    pub mmtk: &'static MMTK<DummyVM>
}

impl FixtureContent for MMTKSingleton {
    fn create() -> Self {
        const MB: usize = 1024 * 1024;
        // 1MB heap
        mmtk_init(MB);
        mmtk_initialize_collection(VMThread::UNINITIALIZED);

        MMTKSingleton {
            mmtk: &crate::SINGLETON,
        }
    }
}

pub struct TwoObjects {
    pub objref1: ObjectReference,
    pub objref2: ObjectReference,
}

impl FixtureContent for TwoObjects {
    fn create() -> Self {
        const MB: usize = 1024 * 1024;
        // 1MB heap
        mmtk_init(MB);
        mmtk_initialize_collection(VMThread::UNINITIALIZED);
        // Make sure GC does not run during test.
        mmtk_disable_collection();
        let handle = mmtk_bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));

        let size = 128;
        let semantics = AllocationSemantics::Default;

        let addr = mmtk_alloc(handle, size, 8, 0, semantics);
        assert!(!addr.is_zero());

        let objref1 = ObjectReference::from_raw_address(addr.add(OBJECT_REF_OFFSET));
        mmtk_post_alloc(handle, objref1, size, semantics);

        let objref2 = ObjectReference::from_raw_address(addr.add(OBJECT_REF_OFFSET));
        mmtk_post_alloc(handle, objref2, size, semantics);

        TwoObjects { objref1, objref2 }
    }
}
