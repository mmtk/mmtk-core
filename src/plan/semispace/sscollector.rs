use ::plan::{phase, Phase};
use ::plan::Allocator as AllocationType;
use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::semispace;
use ::plan::semispace::PLAN;
use ::plan::TraceLocal;
use ::policy::copyspace::CopySpace;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::util::alloc::BumpAllocator;
use ::util::forwarding_word::clear_forwarding_bits;
use ::util::heap::{MonotonePageResource, PageResource};
use ::util::reference_processor::*;
use ::vm::{Scanning, VMScanning};
use libc::c_void;
use super::sstracelocal::SSTraceLocal;

/// per-collector thread behavior and state for the SS plan
pub struct SSCollector {
    pub tls: *mut c_void,
    // CopyLocal
    pub ss: BumpAllocator<MonotonePageResource<CopySpace>>,
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
            trace: SSTraceLocal::new(PLAN.get_sstrace()),

            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
        }
    }

    fn init(&mut self, tls: *mut c_void) {
        self.tls = tls;
        self.ss.tls = tls;
        self.trace.init(tls);
    }

    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize,
                  allocator: AllocationType) -> Address {
        self.ss.alloc(bytes, align, offset)
    }

    fn run(&mut self, tls: *mut c_void) {
        self.tls = tls;
        loop {
            self.park();
            self.collect();
        }
    }

    fn collection_phase(&mut self, tls: *mut c_void, phase: &Phase, primary: bool) {
        println!("t {:?} cp {:?}", tls, phase);
        match phase {
            &Phase::Prepare => { self.ss.rebind(Some(semispace::PLAN.tospace())) }
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
                if primary {
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
                self.trace.release();
                debug_assert!(self.trace.is_empty());
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

impl ParallelCollector for SSCollector {
    type T = SSTraceLocal;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        // FIXME use reference instead of cloning everything
        phase::begin_new_phase_stack(self.tls, (phase::Schedule::Complex, ::plan::plan::COLLECTION.clone()))
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
