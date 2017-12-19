use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::bumpallocator::BumpAllocator;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::Address;
use ::util::alloc::allocator::Allocator;

#[repr(C)]
pub struct NoGCMutator<'a> {
    // CopyLocal
    ss: BumpAllocator<'a, ImmortalSpace>
}

impl<'a> MutatorContext<'a, ImmortalSpace> for NoGCMutator<'a> {
    fn new(thread_id: usize, space: &'a ImmortalSpace) -> Self {
        NoGCMutator {
            ss: BumpAllocator::new(thread_id, space)
        }
    }

    fn collection_phase(&mut self, phase: Phase, primary: bool) {
        unimplemented!();
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.ss.alloc(size, align, offset)
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address {
        self.ss.alloc_slow(size, align, offset)
    }
}