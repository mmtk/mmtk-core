use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::semispace;
use ::plan::Phase;
use ::plan::TraceLocal;

use ::util::alloc::Allocator;
use ::util::alloc::BumpAllocator;
use ::util::{Address, ObjectReference};

use ::policy::copyspace::CopySpace;

use ::vm::VMScanning;
use ::vm::Scanning;

use super::sstracelocal::SSTraceLocal;

/// per-collector thread behavior and state for the SS plan
pub struct SSCollector<'a> {
    pub id: usize,
    // CopyLocal
    pub ss: BumpAllocator<'a, CopySpace>,
    trace: SSTraceLocal,

    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'a ParallelCollectorGroup<SSCollector<'a>>>,
}

impl<'a> CollectorContext for SSCollector<'a> {
    fn new() -> Self {
        SSCollector {
            id: 0,
            ss: BumpAllocator::new(0,None),
            trace: SSTraceLocal::new(),

            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

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
            Phase::Prepare => { self.ss.rebind(Some(semispace::PLAN.tospace())) }
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

impl<'a> ParallelCollector for SSCollector<'a> {
    fn collect(&self) {
        unimplemented!();
    }
    fn get_current_trace<T: TraceLocal>(&self) -> T {
        unimplemented!()
    }
    fn parallel_worker_count(&self) -> usize {
        self.group.unwrap().active_worker_count()
    }
    fn parallel_worker_ordinal(&self) -> usize {
        self.worker_ordinal
    }
    fn rendezvous(&self) -> usize {
        self.group.unwrap().rendezvous()
    }

    fn get_last_trigger_count(&self) -> usize {
        self.last_trigger_count
    }
    fn set_last_trigger_count(&mut self, val: usize) {
        self.last_trigger_count = val;
    }
    fn increment_last_trigger_count(&mut self) {
        self.last_trigger_count += 1;
    }

    fn set_group(&mut self, group: *const ParallelCollectorGroup<Self>) {
        self.group = Some ( unsafe {&*group} );
    }
    fn set_worker_ordinal(&mut self, ordinal: usize) {
        self.worker_ordinal = ordinal;
    }
}