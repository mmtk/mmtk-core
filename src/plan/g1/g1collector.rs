use ::plan::{phase, Phase};
use ::plan::Allocator as AllocationType;
use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::g1;
use ::plan::g1::PLAN;
use ::plan::TraceLocal;
use ::policy::copyspace::CopySpace;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::util::alloc::RegionAllocator;
use ::util::forwarding_word::clear_forwarding_bits;
use ::util::heap::{MonotonePageResource, PageResource};
use ::util::reference_processor::*;
use ::vm::{Scanning, VMScanning};
use libc::c_void;
use super::g1tracelocal::{G1TraceLocal, TraceKind};
use ::plan::selected_plan::SelectedConstraints;

/// per-collector thread behavior and state for the SS plan
pub struct G1Collector {
    pub tls: *mut c_void,
    // CopyLocal
    pub rs: RegionAllocator,
    mark_trace: G1TraceLocal,
    evacuate_trace: G1TraceLocal,
    current_trace: TraceKind,
    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'static ParallelCollectorGroup<G1Collector>>,
}

impl CollectorContext for G1Collector {
    fn new() -> Self {
        G1Collector {
            tls: 0 as *mut c_void,
            rs: RegionAllocator::new(0 as *mut c_void, &PLAN.region_space),
            mark_trace: G1TraceLocal::new(TraceKind::Mark, &PLAN.mark_trace),
            evacuate_trace: G1TraceLocal::new(TraceKind::Evacuate, &PLAN.evacuate_trace),
            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
            current_trace: TraceKind::Mark,
        }
    }

    fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
        self.rs.tls = tls;
        self.mark_trace.init(tls);
        self.evacuate_trace.init(tls);
    }

    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        self.rs.alloc(bytes, align, offset)
    }

    fn run(&mut self, tls: *mut c_void) {
        self.tls = tls;
        loop {
            self.park();
            self.collect();
        }
    }

    fn collection_phase(&mut self, tls: *mut c_void, phase: &Phase, primary: bool) {
        println!("Collector {:?}", phase);
        match phase {
            &Phase::StackRoots => {
                trace!("Computing thread roots");
                VMScanning::compute_thread_roots(&mut self.mark_trace, self.tls);
                trace!("Thread roots complete");
            }
            &Phase::Roots => {
                trace!("Computing global roots");
                match self.current_trace {
                    TraceKind::Mark => {
                        VMScanning::compute_global_roots(&mut self.mark_trace, self.tls);
                        VMScanning::compute_static_roots(&mut self.mark_trace, self.tls);
                        if super::g1::SCAN_BOOT_IMAGE {
                            VMScanning::compute_bootimage_roots(&mut self.mark_trace, self.tls);
                        }
                    },
                    TraceKind::Evacuate => {
                        VMScanning::compute_global_roots(&mut self.evacuate_trace, self.tls);
                        VMScanning::compute_static_roots(&mut self.evacuate_trace, self.tls);
                        if super::g1::SCAN_BOOT_IMAGE {
                            VMScanning::compute_bootimage_roots(&mut self.evacuate_trace, self.tls);
                        }
                    },
                }
            }
            &Phase::SoftRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    match self.current_trace {
                        TraceKind::Mark => scan_soft_refs(&mut self.mark_trace, tls),
                        TraceKind::Evacuate => scan_soft_refs(&mut self.evacuate_trace, tls),
                    }
                }
            }
            &Phase::WeakRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    match self.current_trace {
                        TraceKind::Mark => scan_weak_refs(&mut self.mark_trace, tls),
                        TraceKind::Evacuate => scan_weak_refs(&mut self.evacuate_trace, tls),
                    }
                }
            }
            &Phase::Finalizable => {
                if primary {
                    // FIXME
                }
            }
            &Phase::PhantomRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    match self.current_trace {
                        TraceKind::Mark => scan_phantom_refs(&mut self.mark_trace, tls),
                        TraceKind::Evacuate => scan_phantom_refs(&mut self.evacuate_trace, tls),
                    }
                }
            }
            &Phase::ForwardRefs => {
                if primary && SelectedConstraints::NEEDS_FORWARD_AFTER_LIVENESS {
                    match self.current_trace {
                        TraceKind::Mark => forward_refs(&mut self.mark_trace),
                        TraceKind::Evacuate => forward_refs(&mut self.evacuate_trace),
                    }
                }
            }
            &Phase::ForwardFinalizable => {
                if primary {
                    // FIXME
                }
            }
            &Phase::Complete => {
                debug_assert!(self.mark_trace.is_empty());
                debug_assert!(self.evacuate_trace.is_empty());
            }
            &Phase::Prepare => {
                self.current_trace = TraceKind::Mark;
                self.rs.reset()
            }
            &Phase::Closure => {
                self.mark_trace.complete_trace();
                debug_assert!(self.mark_trace.is_empty());
            }
            &Phase::Release => {
                debug_assert!(self.mark_trace.is_empty());
                self.mark_trace.release();
                debug_assert!(self.mark_trace.is_empty());
            }
            &Phase::EvacuatePrepare => {
                self.current_trace = TraceKind::Evacuate;
                self.rs.reset()
            }
            &Phase::EvacuateClosure => {
                self.evacuate_trace.complete_trace();
                debug_assert!(self.evacuate_trace.is_empty());
            }
            &Phase::EvacuateRelease => {
                debug_assert!(self.evacuate_trace.is_empty());
                self.evacuate_trace.release();
                debug_assert!(self.evacuate_trace.is_empty());
            }
            _ => { panic!("Per-collector phase not handled") }
        }
    }

    fn get_tls(&self) -> *mut c_void {
        self.tls
    }

    fn post_copy(&self, object: ObjectReference, rvm_type: Address, bytes: usize, allocator: ::plan::Allocator) {
        clear_forwarding_bits(object);
        match allocator {
            ::plan::Allocator::Default => {}
            _ => {
                panic!("Currently we can't copy to other spaces other than copyspace")
            }
        }
    }
}

impl ParallelCollector for G1Collector {
    type T = G1TraceLocal;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        // FIXME use reference instead of cloning everything
        phase::begin_new_phase_stack(self.tls, (phase::Schedule::Complex, g1::g1::COLLECTION.clone()))
    }

    fn get_current_trace(&mut self) -> &mut G1TraceLocal {
        match self.current_trace {
            TraceKind::Mark => &mut self.mark_trace,
            TraceKind::Evacuate => &mut self.evacuate_trace,
        }
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
