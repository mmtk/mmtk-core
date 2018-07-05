use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::BumpAllocator;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::plan::semispace;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::plan::plan;
use ::vm::{Collection, VMCollection};
use ::util::heap::{PageResource, MonotonePageResource};
use ::plan::semispace::PLAN;

#[repr(C)]
pub struct SSMutator {
    // CopyLocal
    ss: BumpAllocator<MonotonePageResource<CopySpace>>,
    vs: BumpAllocator<MonotonePageResource<ImmortalSpace>>,
}

impl MutatorContext for SSMutator {
    fn collection_phase(&mut self, thread_id: usize, phase: &Phase, primary: bool) {
        match phase {
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.ss.thread_id, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {}
            &Phase::Release => {
                // rebind the allocation bump pointer to the appropriate semispace
                self.ss.rebind(Some(semispace::PLAN.tospace()));
            }
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.tospace() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc(size, align, offset) }
            _ => { self.vs.alloc(size, align, offset) }
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _,
                      "bumpallocator {:?} holds wrong space, ss.space: {:?}, tospace: {:?}",
                      self as *const _,
                      self.ss.get_space().unwrap() as *const _,
                      PLAN.tospace() as *const _);
        match allocator {
            AllocationType::Default => { self.ss.alloc_slow(size, align, offset) }
            _ => { self.vs.alloc_slow(size, align, offset) }
        }
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        debug_assert!(self.ss.get_space().unwrap() as *const _ == PLAN.tospace() as *const _);
        match allocator {
            AllocationType::Default => {}
            _ => {
                // FIXME: data race on immortalspace.mark_state !!!
                let unsync = unsafe { &*PLAN.unsync.get() };
                unsync.versatile_space.initialize_header(refer);
            }
        }
    }

    fn get_thread_id(&self) -> usize {
        debug_assert!(self.ss.thread_id == self.vs.thread_id);
        self.ss.thread_id
    }
}

impl SSMutator {
    pub fn new(thread_id: usize, space: &'static CopySpace, versatile_space: &'static ImmortalSpace) -> Self {
        SSMutator {
            ss: BumpAllocator::new(thread_id, Some(space)),
            vs: BumpAllocator::new(thread_id, Some(versatile_space)),
        }
    }
}