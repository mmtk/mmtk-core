// Some tests are conditionally compiled. So not all the code in this module will be used. We simply allow dead code in this module.
#![allow(dead_code)]

use atomic_refcell::AtomicRefCell;
use std::sync::Mutex;
use std::sync::Once;

use crate::memory_manager;
use crate::util::test_util::mock_vm::MockVM;
use crate::util::{ObjectReference, VMMutatorThread, VMThread};
use crate::AllocationSemantics;
use crate::MMTKBuilder;
use crate::MMTK;

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

    pub fn with_fixture_mut<F: Fn(&mut T)>(&self, func: F) {
        self.once.call_once(|| {
            let content = Box::new(T::create());
            let mut borrow = self.content.borrow_mut();
            *borrow = Some(content);
        });
        let mut borrow = self.content.borrow_mut();
        func(borrow.as_mut().unwrap())
    }
}

impl<T: FixtureContent> Default for Fixture<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// SerialFixture ensures all `with_fixture()` calls will be executed serially.
pub struct SerialFixture<T: FixtureContent> {
    content: Mutex<Option<Box<T>>>,
}

impl<T: FixtureContent> SerialFixture<T> {
    pub fn new() -> Self {
        Self {
            content: Mutex::new(None),
        }
    }

    pub fn with_fixture<F: Fn(&T)>(&self, func: F) {
        let mut c = self.content.lock().unwrap();
        if c.is_none() {
            *c = Some(Box::new(T::create()));
        }
        func(c.as_ref().unwrap())
    }

    pub fn with_fixture_mut<F: Fn(&mut T)>(&self, func: F) {
        let mut c = self.content.lock().unwrap();
        if c.is_none() {
            *c = Some(Box::new(T::create()));
        }
        func(c.as_mut().unwrap())
    }

    pub fn with_fixture_expect_benign_panic<
        F: Fn(&T) + std::panic::UnwindSafe + std::panic::RefUnwindSafe,
    >(
        &self,
        func: F,
    ) {
        let res = {
            // The lock will be dropped at the end of the block. So panic won't poison the lock.
            let mut c = self.content.lock().unwrap();
            if c.is_none() {
                *c = Some(Box::new(T::create()));
            }

            std::panic::catch_unwind(|| func(c.as_ref().unwrap()))
        };
        // We do not hold the lock now. It is safe to resume now.
        if let Err(e) = res {
            std::panic::resume_unwind(e);
        }
    }
}

impl<T: FixtureContent> Default for SerialFixture<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MMTKFixture {
    pub mmtk: &'static MMTK<MockVM>,
}

impl FixtureContent for MMTKFixture {
    fn create() -> Self {
        Self::create_with_builder(
            |builder| {
                const MB: usize = 1024 * 1024;
                builder
                    .options
                    .gc_trigger
                    .set(crate::util::options::GCTriggerSelector::FixedHeapSize(MB));
            },
            true,
        )
    }
}

impl MMTKFixture {
    pub fn create_with_builder<F>(with_builder: F, initialize_collection: bool) -> Self
    where
        F: FnOnce(&mut MMTKBuilder),
    {
        let mut builder = MMTKBuilder::new();
        with_builder(&mut builder);

        let mmtk = memory_manager::mmtk_init(&builder);
        let mmtk_ptr = Box::into_raw(mmtk);
        let mmtk_static: &'static MMTK<MockVM> = unsafe { &*mmtk_ptr };

        if initialize_collection {
            memory_manager::initialize_collection(mmtk_static, VMThread::UNINITIALIZED);
        }

        MMTKFixture { mmtk: mmtk_static }
    }
}

impl Drop for MMTKFixture {
    fn drop(&mut self) {
        let mmtk_ptr: *const MMTK<MockVM> = self.mmtk as _;
        let _ = unsafe { Box::from_raw(mmtk_ptr as *mut MMTK<MockVM>) };
    }
}

use crate::plan::Mutator;

pub struct MutatorFixture {
    mmtk: MMTKFixture,
    pub mutator: Box<Mutator<MockVM>>,
}

impl FixtureContent for MutatorFixture {
    fn create() -> Self {
        const MB: usize = 1024 * 1024;
        Self::create_with_heapsize(MB)
    }
}

impl MutatorFixture {
    pub fn create_with_heapsize(size: usize) -> Self {
        let mmtk = MMTKFixture::create_with_builder(
            |builder| {
                builder
                    .options
                    .gc_trigger
                    .set(crate::util::options::GCTriggerSelector::FixedHeapSize(size));
            },
            true,
        );
        let mutator =
            memory_manager::bind_mutator(mmtk.mmtk, VMMutatorThread(VMThread::UNINITIALIZED));
        Self { mmtk, mutator }
    }

    pub fn create_with_builder<F>(with_builder: F) -> Self
    where
        F: FnOnce(&mut MMTKBuilder),
    {
        let mmtk = MMTKFixture::create_with_builder(with_builder, true);
        let mutator =
            memory_manager::bind_mutator(mmtk.mmtk, VMMutatorThread(VMThread::UNINITIALIZED));
        Self { mmtk, mutator }
    }

    pub fn mmtk(&self) -> &'static MMTK<MockVM> {
        self.mmtk.mmtk
    }
}

unsafe impl Send for MutatorFixture {}

pub struct SingleObject {
    pub objref: ObjectReference,
    mutator: MutatorFixture,
}

impl FixtureContent for SingleObject {
    fn create() -> Self {
        use crate::vm::object_model::ObjectModel;
        let mut mutator = MutatorFixture::create();

        // A relatively small object, typical for Ruby.
        let size = 40;
        let semantics = AllocationSemantics::Default;

        let addr = memory_manager::alloc(&mut mutator.mutator, size, 8, 0, semantics);
        assert!(!addr.is_zero());

        let objref = MockVM::address_to_ref(addr);
        memory_manager::post_alloc(&mut mutator.mutator, objref, size, semantics);

        SingleObject { objref, mutator }
    }
}

impl SingleObject {
    pub fn mutator(&self) -> &Mutator<MockVM> {
        &self.mutator.mutator
    }

    pub fn mutator_mut(&mut self) -> &mut Mutator<MockVM> {
        &mut self.mutator.mutator
    }
}

pub struct TwoObjects {
    pub objref1: ObjectReference,
    pub objref2: ObjectReference,
    mutator: MutatorFixture,
}

impl FixtureContent for TwoObjects {
    fn create() -> Self {
        use crate::vm::object_model::ObjectModel;
        let mut mutator = MutatorFixture::create();

        let size = 128;
        let semantics = AllocationSemantics::Default;

        let addr1 = memory_manager::alloc(&mut mutator.mutator, size, 8, 0, semantics);
        assert!(!addr1.is_zero());

        let objref1 = MockVM::address_to_ref(addr1);
        memory_manager::post_alloc(&mut mutator.mutator, objref1, size, semantics);

        let addr2 = memory_manager::alloc(&mut mutator.mutator, size, 8, 0, semantics);
        assert!(!addr2.is_zero());

        let objref2 = MockVM::address_to_ref(addr2);
        memory_manager::post_alloc(&mut mutator.mutator, objref2, size, semantics);

        TwoObjects {
            objref1,
            objref2,
            mutator,
        }
    }
}
