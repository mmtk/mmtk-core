use ::policy::copyspace::CopySpace;
use ::util::alloc::BumpAllocator;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::plan::semispace;
use ::util::Address;
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;

#[repr(C)]
pub struct SSMutator<'a> {
    thread_id: usize,
    // CopyLocal
    ss: BumpAllocator<'a, CopySpace>
}

impl<'a> MutatorContext for SSMutator<'a> {
    fn collection_phase(&mut self, phase: Phase, primary: bool) {
        if let Phase::Prepare = phase {
            self.ss.rebind(Some(semispace::PLAN.tospace()));
        }
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        self.ss.alloc(size, align, offset)
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        self.ss.alloc_slow(size, align, offset)
    }
}

impl<'a> SSMutator<'a> {
    pub fn new(thread_id: usize, space: &'a CopySpace) -> Self {
        SSMutator {
            thread_id,
            ss: BumpAllocator::new(thread_id, Some(space)),
        }
    }
}