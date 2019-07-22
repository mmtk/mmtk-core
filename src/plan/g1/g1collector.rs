use ::plan::{phase, Phase};
use ::plan::Allocator as AllocationType;
use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::g1;
use ::plan::g1::{PLAN, DEBUG};
use ::plan::TraceLocal;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::util::alloc::RegionAllocator;
use ::util::forwarding_word::clear_forwarding_bits;
use ::util::reference_processor::*;
use ::vm::{Scanning, VMScanning};
use libc::c_void;
use super::g1tracelocal::{G1TraceLocal, TraceKind};
use ::plan::selected_plan::SelectedConstraints;
use util::alloc::LargeObjectAllocator;

/// per-collector thread behavior and state for the SS plan
pub struct G1Collector {
    pub tls: *mut c_void,
    rs: RegionAllocator,
    los: LargeObjectAllocator,
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
            los: LargeObjectAllocator::new(0 as *mut c_void, Some(PLAN.get_los())),
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
        self.los.tls = tls;
        self.mark_trace.init(tls);
        self.evacuate_trace.init(tls);
    }

    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        match allocator {
            AllocationType::Los => self.los.alloc(bytes, align, offset),
            AllocationType::Default => self.rs.alloc(bytes, align, offset),
            _ => unreachable!(),
        }
    }

    fn run(&mut self, tls: *mut c_void) {
        self.tls = tls;
        loop {
            self.park();
            self.collect();
        }
    }

    fn collection_phase(&mut self, tls: *mut c_void, phase: &Phase, primary: bool) {
        if DEBUG {
            println!("Collector {:?}", phase);
        }
        match phase {
            &Phase::StackRoots => {
                trace!("Computing thread roots");
                let tls = self.tls;
                VMScanning::compute_thread_roots(self.get_current_trace(), tls);
                trace!("Thread roots complete");
            }
            &Phase::Roots => {
                trace!("Computing global roots");
                let tls = self.tls;
                let trace = self.get_current_trace();
                VMScanning::compute_global_roots(trace, tls);
                VMScanning::compute_static_roots(trace, tls);
                if super::g1::SCAN_BOOT_IMAGE {
                    VMScanning::compute_bootimage_roots(trace, tls);
                }
            }
            &Phase::SoftRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    scan_soft_refs(self.get_current_trace(), tls);
                }
            }
            &Phase::WeakRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    scan_weak_refs(self.get_current_trace(), tls);
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
                    scan_phantom_refs(self.get_current_trace(), tls);
                }
            }
            &Phase::ForwardRefs => {
                if primary && SelectedConstraints::NEEDS_FORWARD_AFTER_LIVENESS {
                    forward_refs(self.get_current_trace());
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
            ::plan::Allocator::Los => {
                PLAN.los.initialize_header(object, false);
            }
            _ => unreachable!()
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
