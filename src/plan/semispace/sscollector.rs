use ::plan::{phase, Phase};
use ::plan::Allocator as AllocationType;
use ::plan::CollectorContext;
use ::plan::ParallelCollector;
use ::plan::ParallelCollectorGroup;
use ::plan::TraceLocal;
use ::plan::phase::PhaseManager;
use ::policy::copyspace::CopySpace;
use ::util::{Address, ObjectReference};
use ::util::alloc::Allocator;
use ::util::alloc::{BumpAllocator, LargeObjectAllocator};
use ::util::forwarding_word::clear_forwarding_bits;
use ::util::heap::MonotonePageResource;
use ::util::reference_processor::*;
use ::vm::Scanning;
use super::sstracelocal::SSTraceLocal;
use ::plan::selected_plan::SelectedConstraints;
use util::OpaquePointer;
use plan::semispace::SemiSpace;
use plan::phase::ScheduledPhase;
use mmtk::MMTK;
use vm::VMBinding;

/// per-collector thread behavior and state for the SS plan
pub struct SSCollector<VM: VMBinding> {
    pub tls: OpaquePointer,
    // CopyLocal
    pub ss: BumpAllocator<VM, MonotonePageResource<VM, CopySpace<VM>>>,
    los: LargeObjectAllocator<VM>,
    trace: SSTraceLocal<VM>,

    last_trigger_count: usize,
    worker_ordinal: usize,
    group: Option<&'static ParallelCollectorGroup<VM, SSCollector<VM>>>,

    plan: &'static SemiSpace<VM>,
    phase_manager: &'static PhaseManager,
    reference_processors: &'static ReferenceProcessors,
}

impl<VM: VMBinding> CollectorContext<VM> for SSCollector<VM> {
    fn new(mmtk: &'static MMTK<VM>) -> Self {
        SSCollector {
            tls: OpaquePointer::UNINITIALIZED,
            ss: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
            los: LargeObjectAllocator::new(OpaquePointer::UNINITIALIZED, Some(mmtk.plan.get_los()), &mmtk.plan),
            trace: SSTraceLocal::new(&mmtk.plan),

            last_trigger_count: 0,
            worker_ordinal: 0,
            group: None,
            plan: &mmtk.plan,
            phase_manager: &mmtk.phase_manager,
            reference_processors: &mmtk.reference_processors,
        }
    }

    fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
        self.ss.tls = tls;
        self.los.tls = tls;
        self.trace.init(tls);
    }

    fn alloc_copy(&mut self, _original: ObjectReference, bytes: usize, align: usize, offset: isize,
                  allocator: AllocationType) -> Address {
        match allocator {
            ::plan::Allocator::Los => self.los.alloc(bytes, align, offset),
            _ => self.ss.alloc(bytes, align, offset)
        }

    }

    fn run(&mut self, tls: OpaquePointer) {
        self.tls = tls;
        loop {
            self.park();
            self.collect();
        }
    }

    fn collection_phase(&mut self, _tls: OpaquePointer, phase: &Phase, primary: bool) {
        match phase {
            &Phase::Prepare => { self.ss.rebind(Some(self.plan.tospace())) }
            &Phase::StackRoots => {
                trace!("Computing thread roots");
                VM::VMScanning::compute_thread_roots(&mut self.trace, self.tls);
                trace!("Thread roots complete");
            }
            &Phase::Roots => {
                trace!("Computing global roots");
                VM::VMScanning::compute_global_roots(&mut self.trace, self.tls);
                trace!("Computing static roots");
                VM::VMScanning::compute_static_roots(&mut self.trace, self.tls);
                trace!("Finished static roots");
                if super::ss::SCAN_BOOT_IMAGE {
                    trace!("Scanning boot image");
                    VM::VMScanning::compute_bootimage_roots(&mut self.trace, self.tls);
                    trace!("Finished boot image");
                }
            }
            &Phase::SoftRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    self.reference_processors.scan_soft_refs::<VM, SSTraceLocal<VM>>(&mut self.trace, self.tls)
                }
            }
            &Phase::WeakRefs => {
                if primary {
                    // FIXME Clear refs if noReferenceTypes is true
                    self.reference_processors.scan_weak_refs::<VM, SSTraceLocal<VM>>(&mut self.trace, self.tls)
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
                    self.reference_processors.scan_phantom_refs::<VM, SSTraceLocal<VM>>(&mut self.trace, self.tls)
                }
            }
            &Phase::ForwardRefs => {
                if primary && SelectedConstraints::NEEDS_FORWARD_AFTER_LIVENESS {
                    self.reference_processors.forward_refs::<VM, SSTraceLocal<VM>>(&mut self.trace)
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
                debug_assert!(self.trace.is_empty());
                self.trace.release();
                debug_assert!(self.trace.is_empty());
            }
            _ => { panic!("Per-collector phase not handled") }
        }
    }

    fn get_tls(&self) -> OpaquePointer {
        self.tls
    }

    fn post_copy(&self, object: ObjectReference, _rvm_type: Address, _bytes: usize, allocator: ::plan::Allocator) {
        clear_forwarding_bits::<VM>(object);
        match allocator {
            ::plan::Allocator::Default => {}
            ::plan::Allocator::Los => {
                self.los.get_space().unwrap().initialize_header(object, false);
            }
            _ => {
                panic!("Currently we can't copy to other spaces other than copyspace")
            }
        }
    }
}

impl<VM: VMBinding> ParallelCollector<VM> for SSCollector<VM> {
    type T = SSTraceLocal<VM>;

    fn park(&mut self) {
        self.group.unwrap().park(self);
    }

    fn collect(&self) {
        // FIXME use reference instead of cloning everything
        self.phase_manager.begin_new_phase_stack::<VM>(self.tls, ScheduledPhase::new(phase::Schedule::Complex, self.phase_manager.collection_phase.clone()))
    }

    fn get_current_trace(&mut self) -> &mut SSTraceLocal<VM> {
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

    // See ParallelCollector.set_group()
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn set_group(&mut self, group: *const ParallelCollectorGroup<VM, Self>) {
        self.group = Some(unsafe { &*group });
    }

    fn set_worker_ordinal(&mut self, ordinal: usize) {
        self.worker_ordinal = ordinal;
    }
}
