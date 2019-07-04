use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::policy::largeobjectspace::LargeObjectSpace;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::plan::plan;
use ::vm::{Collection, VMCollection};
use ::util::heap::{PageResource, MonotonePageResource};
use ::plan::gencopy::PLAN;
use ::util::queue::LocalQueue;
use libc::c_void;
use policy::space::Space;

#[repr(C)]
pub struct SSMutator {
    // CopyLocal
    ss: BumpAllocator<MonotonePageResource<CopySpace>>,
    vs: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
    // los: LargeObjectAllocator,
    remset: Box<LocalQueue<'static, Address>>,
}

impl MutatorContext for SSMutator {
    fn collection_phase(&mut self, _tls: *mut c_void, phase: &Phase, _primary: bool) {
        // println!("Mutator {:?}", phase);
        match phase {
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.ss.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {
                self.ss.reset();
                self.flush_remembered_sets();
                debug_assert!(self.remset.is_empty());
            }
            &Phase::ValidatePrepare => {
                self.ss.reset();
            }
            &Phase::Release => {
                self.ss.reset();
                debug_assert!(self.remset.is_empty());
            }
            &Phase::ValidateRelease => {
                self.ss.reset();
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.nursery_space() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, nursery_space: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.nursery_space() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc(size, align, offset) }
            // AllocationType::Los => { unimplemented!() }
            _ => { self.vs.alloc(size, align, offset) }
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.nursery_space() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, nursery_space: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.nursery_space() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc_slow(size, align, offset) }
            // AllocationType::Los => { self.los.alloc(size, align, offset) }
            _ => { self.vs.alloc_slow(size, align, offset) }
        }
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.nursery_space() as *const _);
        match allocator {
            AllocationType::Default => {
                // println!("Alloc nursery {:?} tib={:?}", refer, type_refer);
            }
            // AllocationType::Los => {
            //     // FIXME: data race on immortalspace.mark_state !!!
            //     let unsync = unsafe { &*PLAN.unsync.get() };
            //     unsync.los.initialize_header(refer, true);
            // }
            _ => {
                // FIXME: data race on immortalspace.mark_state !!!
                let unsync = unsafe { &*PLAN.unsync.get() };
                unsync.versatile_space.initialize_header(refer);
            }
        }
    }

    fn get_tls(&self) -> *mut c_void {
        debug_assert!(self.ss.tls == self.vs.tls);
        self.ss.tls
    }

    fn flush_remembered_sets(&mut self) {
        self.remset.flush();
    }

    fn object_reference_write_slow(&mut self, src: ObjectReference, slot: Address, value: ObjectReference) {
        debug_assert!(PLAN.nursery_space().in_space(value), "value={:?}", value);
        debug_assert!(!PLAN.nursery_space().in_space(src));
        self.remset.enqueue(slot);
    }
}

impl SSMutator {
    pub fn new(tls: *mut c_void, space: &'static CopySpace, versatile_space: &'static ImmortalSpace) -> Self {
        SSMutator {
            ss: BumpAllocator::new(tls, Some(space)),
            vs: BumpAllocator::new(tls, Some(versatile_space)),
            // los: LargeObjectAllocator::new(tls, Some(los)),
            remset: box PLAN.remset_pool.spawn_local(),
        }
    }
}