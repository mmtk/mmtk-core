use ::plan::{phase, Phase};
use ::plan::Allocator as AllocationType;
use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::TraceLocal;
use ::policy::copyspace::CopySpace;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::util::forwarding_word::clear_forwarding_bits;
use ::util::heap::{MonotonePageResource};
use ::util::reference_processor::*;
use libc::c_void;
use ::plan::selected_plan::SelectedConstraints;
use vm::*;
use super::PLAN;
use super::sstracelocal::SSTraceLocal;
use super::VERBOSE;

static mut CONTINUE_COLLECTING: bool = false;

/// per-collector thread behavior and state for the SS plan
pub struct SSCollector {
    pub tls: *mut c_void,
    pub ss: BumpAllocator<MonotonePageResource<CopySpace>>,
    los: LargeObjectAllocator,
    trace: SSTraceLocal,
    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'static ParallelCollectorGroup<SSCollector>>,
}

impl CollectorContext for SSCollector {
    fn new() -> Self {
        SSCollector {
            tls: 0 as *mut c_void,
            ss: BumpAllocator::new(0 as *mut c_void, None),
            los: LargeObjectAllocator::new(0 as *mut c_void, Some(PLAN.get_los())),
            trace: SSTraceLocal::new(PLAN.get_sstrace()),
            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

    fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
        self.ss.tls = tls;
        self.los.tls = tls;
        self.trace.init(tls);
    }

    fn get_tls(&self) -> *mut c_void {
        self.tls
    }

    fn alloc_copy(&mut self, _original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: AllocationType) -> Address {
        match allocator {
            ::plan::Allocator::Los => self.los.alloc(bytes, align, offset),
            ::plan::Allocator::Default => self.ss.alloc(bytes, align, offset),
            _ => unreachable!(),
        }
    }

    fn post_copy(&self, object: ObjectReference, _rvm_type: Address, _bytes: usize, allocator: ::plan::Allocator) {
        clear_forwarding_bits(object);
        match allocator {
            ::plan::Allocator::Default => {}
            ::plan::Allocator::Los => {
                PLAN.los.initialize_header(object, false);
            }
            _ => {
                panic!("Currently we can't copy to other spaces other than copyspace")
            }
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
        if VERBOSE {
            println!("Collector {:?}", phase);
        }
        match phase {
            &Phase::FlushCollector => {
                self.trace.process_roots();
                self.trace.flush();
            }
            &Phase::Prepare => {
                self.ss.rebind(Some(PLAN.tospace()))
            }
            &Phase::StackRoots => {
                trace!("Computing thread roots");
                VMScanning::compute_thread_roots(&mut self.trace, self.tls);
                trace!("Thread roots complete");
            }
            &Phase::Roots => {
                trace!("Computing global roots");
                VMScanning::compute_global_roots(&mut self.trace, self.tls);
                trace!("Computing static roots");
                VMScanning::compute_static_roots(&mut self.trace, self.tls);
                trace!("Finished static roots");
                if super::ss::SCAN_BOOT_IMAGE {
                    trace!("Scanning boot image");
                    VMScanning::compute_bootimage_roots(&mut self.trace, self.tls);
                    trace!("Finished boot image");
                }
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
            &Phase::FinalClosure => {
                self.trace.complete_trace();
                debug_assert!(self.trace.is_empty());
            }
            &Phase::Release => {
                debug_assert!(self.trace.is_empty());
                self.trace.release();
                debug_assert!(self.trace.is_empty());
                debug_assert!(self.trace.modbuf.is_empty());
            }
            &Phase::ValidatePrepare => {}
            &Phase::ValidateClosure => {
                self.trace.complete_trace();
                debug_assert!(self.trace.is_empty());
            }
            &Phase::ValidateRelease => {
                debug_assert!(self.trace.is_empty());
                debug_assert!(self.trace.modbuf.is_empty());
                self.trace.release();
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
                self.ss.rebind(Some(PLAN.tospace()));
                debug_assert!(!::plan::plan::gc_in_progress());
                while !self.trace.incremental_trace(100) {
                    if self.group.unwrap().is_aborted() {
                      self.trace.flush();
                    //   println!("Concurrent Collection Aborted!");
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

impl ParallelCollector for SSCollector {
    type T = SSTraceLocal;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        if !phase::is_phase_stack_empty() {
            phase::continue_phase_stack(self.tls);
        } else {
            phase::begin_new_phase_stack(self.tls, (phase::Schedule::Complex, super::ss::COLLECTION.clone()));
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

impl SSCollector {
    fn concurrent_trace_complete(&self) -> bool {
        !PLAN.ss_trace.has_work()
    }
}