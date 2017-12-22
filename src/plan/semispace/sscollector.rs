use ::plan::collector_context::CollectorContext;
use ::util::alloc::bumpallocator::BumpAllocator;
use ::util::{Address, ObjectReference};
use ::plan::Phase;
use ::policy::copyspace::CopySpace;
use ::plan::semispace;
use util::alloc::allocator::Allocator;

/// per-collector thread behavior and state for the SS plan
pub struct SSCollector<'a> {
    id: usize,
    // CopyLocal
    ss: BumpAllocator<'a, CopySpace>,
}

impl<'a> CollectorContext for SSCollector<'a> {
    fn init(&mut self, id: usize) {
        self.id = id;
    }

    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: usize) -> Address {
        self.ss.alloc(bytes, align, offset)
    }

    fn run(&self) {
        self.collect();
    }

    fn collection_phase(&mut self, phase: Phase, primary: bool) {
        if let Phase::Prepare = phase {
            self.ss.rebind(semispace::PLAN.tospace());
        }
    }
}

impl<'a> SSCollector<'a> {
    pub fn new(thread_id: usize, space: &'a CopySpace) -> Self {
        SSCollector {
            id: 0,
            ss: BumpAllocator::new(thread_id, space),
        }
    }

    /// Perform a single garbage collection
    fn collect(&self) {
        unimplemented!()
    }
}

