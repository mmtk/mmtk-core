use ::plan::collector_context::CollectorContext;
use ::util::alloc::bumpallocator::BumpAllocator;
use ::util::{Address, ObjectReference};
use ::plan::Phase;
use ::policy::copyspace::CopySpace;
use ::plan::semispace;
use ::util::alloc::Allocator;
use ::vm::VMScanning;
use ::vm::Scanning;

use super::sstracelocal::SSTraceLocal;

/// per-collector thread behavior and state for the SS plan
pub struct SSCollector<'a> {
    id: usize,
    // CopyLocal
    ss: BumpAllocator<'a, CopySpace>,
    trace: SSTraceLocal,
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
        match phase {
            Phase::Prepare => { self.ss.rebind(semispace::PLAN.tospace()) }
            Phase::StackRoots => {
                VMScanning::compute_thread_roots(&mut self.trace);
            }
            Phase::Roots => {
                VMScanning::compute_global_roots(&mut self.trace);
                VMScanning::compute_static_roots(&mut self.trace);
                if super::ss::SCAN_BOOT_IMAGE {
                    VMScanning::compute_bootimage_roots(&mut self.trace);
                }
            }
            Phase::Closure => { self.trace.complete_trace() }
            Phase::Release => { self.trace.release() }
            _ => {}
        }
    }
}

impl<'a> SSCollector<'a> {
    pub fn new(thread_id: usize, space: &'a CopySpace) -> Self {
        SSCollector {
            id: 0,
            ss: BumpAllocator::new(thread_id, space),
            trace: SSTraceLocal::new(),
        }
    }

    /// Perform a single garbage collection
    fn collect(&self) {
        unimplemented!()
    }
}

