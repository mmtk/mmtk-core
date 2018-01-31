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

#[repr(C)]
pub struct SSMutator<'a> {
    thread_id: usize,
    // CopyLocal
    ss: BumpAllocator<'a, CopySpace>,
    vs: BumpAllocator<'a, ImmortalSpace>,
}

impl<'a> MutatorContext for SSMutator<'a> {
    fn collection_phase(&mut self, thread_id: usize, phase: &Phase, primary: bool) {
        match phase {
            &Phase::Prepare => {
                // rebing the allocation bump pointer to the appropriate semispace
                self.ss.rebind(Some(semispace::PLAN.tospace()));
            }
            &Phase::PrepareStacks => {
                if !plan::stacks_prepared() {
                    unimplemented!("VM.collection.prepareMutator");
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
        match allocator {
            AllocationType::Default => { self.ss.alloc(size, align, offset) }
            _ => { self.vs.alloc(size, align, offset) }
        }
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        match allocator {
            AllocationType::Default => { self.ss.alloc_slow(size, align, offset) }
            _ => { self.vs.alloc_slow(size, align, offset) }
        }
    }
}

impl<'a> SSMutator<'a> {
    pub fn new(thread_id: usize, space: &'a CopySpace, versatile_space: &'a ImmortalSpace) -> Self {
        SSMutator {
            thread_id,
            ss: BumpAllocator::new(thread_id, Some(space)),
            vs: BumpAllocator::new(thread_id, Some(versatile_space)),
        }
    }
}