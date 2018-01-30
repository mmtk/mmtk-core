use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::semispace;
use ::plan::{phase, Phase};
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
            ss: BumpAllocator::new(0, None),
            trace: SSTraceLocal::new(),

            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

    fn init(&mut self, id: usize) {
        self.id = id;
        self.trace.init(id);
    }

    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: usize) -> Address {
        self.ss.alloc(bytes, align, offset)
    }

    fn run(&mut self, thread_id: usize) {
        self.id = thread_id;
        loop {
            self.park();
            self.collect();
        }
    }

    fn collection_phase(&mut self, thread_id: usize, phase: &Phase, primary: bool) {
        match phase {
            &Phase::Prepare => { self.ss.rebind(Some(semispace::PLAN.tospace())) }
            &Phase::StackRoots => {
                VMScanning::compute_thread_roots(&mut self.trace, self.id);
            }
            &Phase::Roots => {
                VMScanning::compute_global_roots(&mut self.trace, self.id);
                VMScanning::compute_static_roots(&mut self.trace, self.id);
                if super::ss::SCAN_BOOT_IMAGE {
                    VMScanning::compute_bootimage_roots(&mut self.trace, self.id);
                }
            }
            &Phase::SoftRefs => {
                // FIXME
            }
            &Phase::WeakRefs => {
                // FIXME
            }
            &Phase::Finalizable => {
                // FIXME
            }
            &Phase::PhantomRefs => {
                // FIXME
            }
            &Phase::ForwardRefs => {
                // FIXME
            }
            &Phase::ForwardFinalizable => {
                // FIXME
            }
            &Phase::Complete => {
                unimplemented!()
            }
            &Phase::Closure => { self.trace.complete_trace() }
            &Phase::Release => { self.trace.release() }
            _ => { panic!("Per-collector phase not handled") }
        }
    }

    fn get_id(&self) -> usize {
        self.id
    }
}

impl<'a> ParallelCollector for SSCollector<'a> {
    type T = SSTraceLocal;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        // FIXME use reference instead of cloning everything
        phase::begin_new_phase_stack(self.id, (phase::Schedule::Complex, ::plan::plan::COLLECTION.clone()))
    }

    fn get_current_trace(&mut self) -> &mut SSTraceLocal {
        &mut self.trace
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
        self.group = Some(unsafe { &*group });
    }

    fn set_worker_ordinal(&mut self, ordinal: usize) {
        self.worker_ordinal = ordinal;
    }
}