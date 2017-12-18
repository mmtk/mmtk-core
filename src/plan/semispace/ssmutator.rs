use ::policy::copyspace::CopySpace;
use ::util::alloc::bumpallocator::BumpAllocator;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::plan::semispace;
use ::util::Address;
use ::util::alloc::allocator::Allocator;

pub struct SSMutator<'a> {
    // CopyLocal
    ss: BumpAllocator<'a, CopySpace>
}

impl<'a> MutatorContext for SSMutator<'a> {
    fn collection_phase(&mut self, phase: Phase, primary: bool) {
        if let Phase::Prepare = phase {
            self.ss.rebind(semispace::PLAN.tospace());
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.ss.alloc(size, align, offset)
    }
}

impl<'a> SSMutator<'a> {
    pub fn new(thread_id: usize, space: &'a CopySpace) -> Self {
        SSMutator {
            ss: BumpAllocator::new(thread_id, space)
        }
    }
}