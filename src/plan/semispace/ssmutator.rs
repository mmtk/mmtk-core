use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::policy::largeobjectspace::LargeObjectSpace;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::plan::semispace;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::plan::plan;
use ::vm::{Collection, VMCollection};
use ::util::heap::{PageResource, MonotonePageResource};
use ::util::OpaquePointer;

use libc::c_void;
use plan::semispace::SemiSpace;

#[repr(C)]
pub struct SSMutator {
    // CopyLocal
    ss: BumpAllocator<MonotonePageResource<CopySpace>>,
    vs: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
    los: LargeObjectAllocator,

    plan: &'static SemiSpace
}

impl MutatorContext for SSMutator {
    fn collection_phase(&mut self, tls: OpaquePointer, phase: &Phase, primary: bool) {
        match phase {
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.ss.tls, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {}
            &Phase::Release => {
                // rebind the allocation bump pointer to the appropriate semispace
                self.ss.rebind(Some(self.plan.tospace()));
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc(size, align, offset) }
            AllocationType::Los => { self.los.alloc(size, align, offset) }
            _ => { self.vs.alloc(size, align, offset) }
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc_slow(size, align, offset) }
            AllocationType::Los => { self.los.alloc(size, align, offset) }
            _ => { self.vs.alloc_slow(size, align, offset) }
        }
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == self.plan.tospace() as *const _);
        match allocator {
            AllocationType::Default => {}
            AllocationType::Los => {
                // FIXME: data race on immortalspace.mark_state !!!
                self.los.get_space().unwrap().initialize_header(refer, true);
            }
            _ => {
                // FIXME: data race on immortalspace.mark_state !!!
                self.vs.get_space().unwrap().initialize_header(refer);
            }
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        debug_assert!(self.ss.tls == self.vs.tls);
        debug_assert!(self.ss.tls == self.los.tls);
        self.ss.tls
    }
}

impl SSMutator {
    pub fn new(tls: OpaquePointer, plan: &'static SemiSpace) -> Self {
        SSMutator {
            ss: BumpAllocator::new(tls, Some(plan.tospace())),
            vs: BumpAllocator::new(tls, Some(plan.get_versatile_space())),
            los: LargeObjectAllocator::new(tls, Some(plan.get_los())),
            plan
        }
    }
}