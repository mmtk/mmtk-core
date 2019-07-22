use ::plan::{phase, Phase};
use ::plan::Allocator as AllocationType;
use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use super::PLAN;
use ::plan::TraceLocal;
use ::util::{Address, ObjectReference};
use ::util::reference_processor::*;
use ::vm::*;
use libc::c_void;
use super::nogctracelocal::NoGCTraceLocal;
use ::plan::selected_plan::SelectedConstraints;
use super::VERBOSE;

/// per-collector thread behavior and state for the SS plan
pub struct NoGCCollector {
    pub tls: *mut c_void,
    trace: NoGCTraceLocal,
    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'static ParallelCollectorGroup<NoGCCollector>>,
}

static mut CONTINUE_COLLECTING: bool = false;

impl CollectorContext for NoGCCollector {
    fn new() -> Self {
        NoGCCollector {
            tls: 0 as *mut c_void,
            trace: NoGCTraceLocal::new(PLAN.get_trace()),
            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

    fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
        self.trace.init(tls);
    }

    fn alloc_copy(&mut self, _original: ObjectReference, _bytes: usize, _align: usize, _offset: isize, _allocator: AllocationType) -> Address {
        unreachable!()
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

    fn collection_phase(&mut self, _tls: *mut c_void, phase: &Phase, primary: bool) {
        if VERBOSE {
            println!("Collector {:?}", phase);
        }
        match phase {
            &Phase::FlushCollector => {
                self.trace.process_roots();
                self.trace.flush();
            }
            &Phase::Prepare => {
                
            }
            &Phase::StackRoots => {
                VMScanning::compute_thread_roots(&mut self.trace, self.tls);
            }
            &Phase::Roots => {
                VMScanning::compute_global_roots(&mut self.trace, self.tls);
                VMScanning::compute_static_roots(&mut self.trace, self.tls);
                VMScanning::compute_bootimage_roots(&mut self.trace, self.tls);
            }
            &Phase::SoftRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    scan_soft_refs(&mut self.trace, self.tls)
                }
            }
            &Phase::WeakRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    scan_weak_refs(&mut self.trace, self.tls)
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
                    scan_phantom_refs(&mut self.trace, self.tls)
                }
            }
            &Phase::ForwardRefs => {
                if primary && SelectedConstraints::NEEDS_FORWARD_AFTER_LIVENESS {
                    forward_refs(&mut self.trace)
                }
            }
            &Phase::ForwardFinalizable => {
                if primary {
                    // FIXME
                }
            }
            &Phase::Complete => {
                debug_assert!(self.trace.is_empty());
            }
            &Phase::Closure => {
                self.trace.complete_trace();
                debug_assert!(self.trace.is_empty());
            }
            &Phase::Release => {
                self.trace.complete_trace();
                self.trace.release();
                debug_assert!(self.trace.is_empty());
                debug_assert!(self.trace.modbuf.is_empty());
            }
            _ => { panic!("Per-collector phase not handled") }
        }
    }

    fn concurrent_collection_phase(&mut self, phase: &Phase) {
        if VERBOSE {
            println!("Concurrent {:?}", phase);
        }
        match phase {
            &Phase::Concurrent(_) => {
                debug_assert!(!::plan::plan::gc_in_progress());
                while !self.trace.incremental_trace(100) {
                    if self.group.unwrap().is_aborted() {
                      self.trace.flush();
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


    fn get_tls(&self) -> *mut c_void {
        self.tls
    }

    fn post_copy(&self, _object: ObjectReference, _rvm_type: Address, _bytes: usize, _allocator: ::plan::Allocator) {
        unreachable!()
    }
}

impl ParallelCollector for NoGCCollector {
    type T = NoGCTraceLocal;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        if !phase::is_phase_stack_empty() {
            phase::continue_phase_stack(self.tls);
        } else {
            phase::begin_new_phase_stack(self.tls, (phase::Schedule::Complex, super::nogc::COLLECTION.clone()));
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

    fn get_current_trace(&mut self) -> &mut NoGCTraceLocal {
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

impl NoGCCollector {
    fn concurrent_trace_complete(&self) -> bool {
        !PLAN.trace.has_work()
    }
}