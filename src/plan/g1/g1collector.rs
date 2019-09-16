use ::plan::{phase, Phase};
use ::plan::Allocator as AllocationType;
use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::g1::{PLAN, VERBOSE};
use ::plan::TraceLocal;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::util::alloc::RegionAllocator;
use ::util::forwarding_word::clear_forwarding_bits;
use ::util::reference_processor::*;
use ::vm::*;
use libc::c_void;
use super::g1tracelocal::{G1TraceLocal, TraceKind};
use ::plan::selected_plan::SelectedConstraints;
use util::alloc::LargeObjectAllocator;
use policy::region::*;
use super::multitracelocal::*;
use super::{G1MarkTraceLocal, G1EvacuateTraceLocal, G1NurseryTraceLocal};
use super::validate::ValidateTraceLocal;

static mut CONTINUE_COLLECTING: bool = false;

/// per-collector thread behavior and state for the SS plan
pub struct G1Collector {
    pub tls: *mut c_void,
    rs_survivor: RegionAllocator,
    rs_old: RegionAllocator,
    los: LargeObjectAllocator,
    trace: G1TraceLocal,
    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'static ParallelCollectorGroup<G1Collector>>,
}

impl CollectorContext for G1Collector {
    fn new() -> Self {
        let unsync = unsafe { &mut *PLAN.unsync.get() };
        G1Collector {
            tls: 0 as *mut c_void,
            rs_survivor: RegionAllocator::new(0 as *mut c_void, &mut unsync.region_space, Gen::Survivor),
            rs_old: RegionAllocator::new(0 as *mut c_void, &mut unsafe { &mut *PLAN.unsync.get() }.region_space, Gen::Old),
            los: LargeObjectAllocator::new(0 as *mut c_void, Some(PLAN.get_los())),
            trace: multitracelocal! {
                G1MarkTraceLocal::new(&PLAN.mark_trace),
                G1EvacuateTraceLocal::new(&PLAN.evacuate_trace),
                G1NurseryTraceLocal::new(&PLAN.evacuate_trace),
                ValidateTraceLocal::<()>::new()
            },
            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

    fn get_tls(&self) -> *mut c_void {
        self.tls
    }

    fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
        self.rs_survivor.tls = tls;
        self.rs_old.tls = tls;
        self.los.tls = tls;
        self.trace.mark_trace_mut().init(tls);
        self.trace.evacuate_trace_mut().init(tls);
        self.trace.nursery_trace_mut().init(tls);
        self.trace.validate_trace_mut().init(tls);
        self.trace.set_active(TraceKind::Mark as _);
    }

    fn alloc_copy(&mut self, _original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        match allocator {
            AllocationType::Los => self.los.alloc(bytes, align, offset),
            AllocationType::G1Survivor => self.rs_survivor.alloc(bytes, align, offset),
            AllocationType::G1Old => self.rs_old.alloc(bytes, align, offset),
            _ => unreachable!(),
        }
    }

    fn post_copy(&self, object: ObjectReference, _rvm_type: Address, bytes: usize, allocator: ::plan::Allocator) {
        clear_forwarding_bits(object);
        match allocator {
            AllocationType::G1Survivor | AllocationType::G1Old => {
                PLAN.region_space.initialize_header(object, bytes, false, !super::ENABLE_REMEMBERED_SETS, false);
            }
            AllocationType::Los => {
                PLAN.los.initialize_header(object, false);
            }
            _ => unreachable!()
        }
    }

    fn run(&mut self, tls: *mut c_void) {
        self.tls = tls;
        loop {
            self.park();
            if self.group.unwrap().concurrent {
                self.concurrent_collect();
            } else {
                self.collect();
            }
        }
    }

    fn collection_phase(&mut self, tls: *mut c_void, phase: &Phase, primary: bool) {
        if VERBOSE && primary {
            println!("Collector {:?}", phase);
        }
        match phase {
            &Phase::FlushCollector => {
                self.trace.process_roots();
                self.trace.mark_trace_mut().flush();
            }
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
                VMScanning::compute_bootimage_roots(trace, tls);
            }
            &Phase::RemSetRoots => {
                debug_assert!(super::ENABLE_REMEMBERED_SETS);
                // debug_assert!(self.trace.activated_trace() == TraceKind::Evacuate);
                if primary {
                    PLAN.region_space.prepare_to_iterate_regions_par();
                }
                self.rendezvous();
                let id = self.worker_ordinal;
                let workers = self.parallel_worker_count();
                PLAN.region_space.iterate_tospace_remset_roots(self.get_current_trace(), id, workers, PLAN.in_nursery);
                self.rendezvous();
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
                debug_assert!(self.trace.mark_trace().is_empty());
                debug_assert!(self.trace.evacuate_trace().is_empty());
            }
            &Phase::Prepare => {
                self.trace.set_active(TraceKind::Mark as _);
                debug_assert!(self.trace.activated_trace() == TraceKind::Mark);
                self.rs_survivor.reset();
                self.rs_old.reset();
            }
            &Phase::Closure => {
                self.trace.complete_trace();
            }
            &Phase::FinalClosure => {
                debug_assert!(self.trace.activated_trace() == TraceKind::Mark);
                self.trace.complete_trace();
                debug_assert!(self.trace.mark_trace().is_empty());
            }
            &Phase::Release => {
                debug_assert!(self.trace.activated_trace() == TraceKind::Mark);
                debug_assert!(self.trace.mark_trace().is_empty());
                self.trace.release();
                self.rs_survivor.reset();
                self.rs_old.reset();
                debug_assert!(self.trace.mark_trace().is_empty());
            }
            &Phase::RefineCards => {
                // debug_assert!(self.trace.activated_trace() == TraceKind::Evacuate);
                let workers = self.parallel_worker_count();
                super::concurrent_refine::collector_refine_all_dirty_cards(self.worker_ordinal, workers);
                self.rendezvous();
                if super::SLOW_ASSERTIONS {
                    if primary {
                        cardtable::get().assert_all_cards_are_not_marked();
                    }
                    self.rendezvous();
                }
            }
            &Phase::EvacuatePrepare => {
                if PLAN.in_nursery {
                    self.trace.set_active(TraceKind::Nursery as _);
                } else {
                    self.trace.set_active(TraceKind::Evacuate as _);
                }
                self.rs_survivor.reset();
                self.rs_old.reset();
            }
            &Phase::EvacuateClosure => {
                self.trace.complete_trace();
                debug_assert!(self.trace.evacuate_trace().is_empty());
            }
            &Phase::EvacuateRelease => {
                debug_assert!(self.trace.evacuate_trace().is_empty());
                self.trace.release();
                self.rs_survivor.reset();
                self.rs_old.reset();
                debug_assert!(self.trace.evacuate_trace().is_empty());
                
                if super::SLOW_ASSERTIONS {
                    if primary {
                        cardtable::get().assert_all_cards_are_not_marked();
                    }
                    self.rendezvous();
                }
            }
            &Phase::ValidatePrepare => {
                self.trace.set_active(TraceKind::Validate as _);
                self.rs_survivor.reset();
                self.rs_old.reset();
            }
            &Phase::ValidateRelease => {
                self.trace.release();
                self.rs_survivor.reset();
                self.rs_old.reset();
            }
            _ => { panic!("Per-collector phase not handled") }
        }
    }

    fn concurrent_collection_phase(&mut self, phase: &Phase) {
        if super::VERBOSE {
            if self.rendezvous() == 0 {
                println!("Concurrent Closure");
            }
            self.rendezvous();
        }
        match phase {
            &Phase::Concurrent(_) => {
                self.trace.set_active(TraceKind::Mark as _);
                debug_assert!(self.trace.activated_trace() == TraceKind::Mark);
                debug_assert!(!::plan::plan::gc_in_progress());
                while !self.trace.mark_trace_mut().incremental_trace(100) {
                    if self.group.unwrap().is_aborted() {
                      self.trace.mark_trace_mut().flush();
                      break;
                    }
                }
                if self.rendezvous() == 0 {
                    unsafe { CONTINUE_COLLECTING = false };
                    if !self.group.unwrap().is_aborted() {
                        /* We are responsible for ensuring termination. */
                        debug!("< requesting mutator flush >");
                        VMCollection::request_mutator_flush(self.tls);
                        debug!("< mutators flushed >");
                        if self.concurrent_trace_complete() {
                          let continue_collecting = ::plan::phase::notify_concurrent_phase_complete();
                          unsafe { CONTINUE_COLLECTING = continue_collecting };
                        } else {
                          unsafe { CONTINUE_COLLECTING = true };
                          ::plan::phase::notify_concurrent_phase_incomplete();
                        }
                    }
                }
                self.rendezvous();
            },
            _ => unreachable!(),
        }
    }
}

impl ParallelCollector for G1Collector {
    type T = G1TraceLocal;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn get_current_trace(&mut self) -> &mut G1TraceLocal {
        &mut self.trace
    }

    fn collect(&self) {
        if !phase::is_phase_stack_empty() {
            phase::continue_phase_stack(self.tls);
        } else {
            if PLAN.in_nursery {
                debug_assert!(super::ENABLE_GENERATIONAL_GC);
                phase::begin_new_phase_stack(self.tls, (phase::Schedule::Complex, super::collection::NURSERY_COLLECTION.clone()));
            } else {
                phase::begin_new_phase_stack(self.tls, (phase::Schedule::Complex, super::collection::COLLECTION.clone()));
            }
        }
    }

    fn concurrent_collect(&mut self) {
        debug_assert!(!::plan::plan::gc_in_progress());
        loop {
            let phase = ::plan::phase::get_concurrent_phase();
            self.concurrent_collection_phase(&phase);
            if !unsafe { CONTINUE_COLLECTING } {
                break;
            }
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

impl G1Collector {
    fn concurrent_trace_complete(&self) -> bool {
        !PLAN.mark_trace.has_work()
    }
}
