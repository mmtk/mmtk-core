use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::BumpAllocator;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::plan::semispace;
use ::util::Address;
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::plan::plan;
use ::vm::{Collection, VMCollection};
use ::util::heap::{PageResource, MonotonePageResource};

#[repr(C)]
pub struct SSMutator<'a> {
    // CopyLocal
    ss: BumpAllocator<'a, CopySpace<'a>, MonotonePageResource<'a, CopySpace<'a>>>,
    vs: BumpAllocator<'a, ImmortalSpace<'a>, MonotonePageResource<'a, ImmortalSpace<'a>>>,
}

impl<'a> MutatorContext for SSMutator<'a> {
    fn collection_phase(&mut self, thread_id: usize, phase: &Phase, primary: bool) {
        match phase {
            &Phase::Prepare => {
                // rebind the allocation bump pointer to the appropriate semispace
                self.ss.rebind(Some(semispace::PLAN.tospace()));
            }
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    VMCollection::prepare_mutator(self.ss.thread_id, self);
                }
                self.flush_remembered_sets();
            }
            &Phase::Prepare => {}
            &Phase::Release => {}
            _ => {
                panic!("Per-mutator phase not handled!")
            }
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        match allocator {
            AllocationType::Default => { self.ss.alloc(size, align, offset) }
            _ => { self.vs.alloc(size, align, offset) }
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        match allocator {
            AllocationType::Default => { self.ss.alloc_slow(size, align, offset) }
            _ => { self.vs.alloc_slow(size, align, offset) }
        }
    }
    fn get_thread_id(&self) -> usize {
        debug_assert!(self.ss.thread_id == self.vs.thread_id);
        self.ss.thread_id
    }
}

impl<'a> SSMutator<'a> {
    pub fn new(thread_id: usize, space: &'a CopySpace, versatile_space: &'a ImmortalSpace) -> Self {
        SSMutator {
            ss: BumpAllocator::new(thread_id, Some(space)),
            vs: BumpAllocator::new(thread_id, Some(versatile_space)),
        }
    }
}