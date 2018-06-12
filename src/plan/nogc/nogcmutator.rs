use ::policy::immortalspace::ImmortalSpace;
use ::util::alloc::BumpAllocator;
use ::plan::mutator_context::MutatorContext;
use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::plan::Allocator as AllocationType;
use ::util::heap::MonotonePageResource;

#[repr(C)]
pub struct NoGCMutator {
    // ImmortalLocal
    nogc: BumpAllocator<MonotonePageResource<ImmortalSpace>>
}

impl MutatorContext for NoGCMutator {
    fn collection_phase(&mut self, thread_id: usize, phase: &Phase, primary: bool) {
        unimplemented!();
    }

    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc({}, {}, {}, {:?})", size, align, offset, allocator);
        self.nogc.alloc(size, align, offset)
    }

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        trace!("MutatorContext.alloc_slow({}, {}, {}, {:?})", size, align, offset, allocator);
        self.nogc.alloc_slow(size, align, offset)
    }

    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize, allocator: AllocationType) {
        unimplemented!()
    }

    fn get_thread_id(&self) -> usize {
        self.nogc.thread_id
    }
}

impl NoGCMutator {
    pub fn new(thread_id: usize, space: &'static ImmortalSpace) -> Self {
        NoGCMutator {
            nogc: BumpAllocator::new(thread_id, Some(space)),
        }
    }
}