//! MMTk instance.
use crate::global_state::{GcStatus, GlobalState};
use crate::plan::gc_requester::GCRequester;
use crate::plan::CreateGeneralPlanArgs;
use crate::plan::Plan;
use crate::policy::sft_map::{create_sft_map, SFTMap};
use crate::scheduler::GCWorkScheduler;

#[cfg(feature = "analysis")]
use crate::util::analysis::AnalysisManager;
#[cfg(feature = "extreme_assertions")]
use crate::util::edge_logger::EdgeLogger;
use crate::util::finalizable_processor::FinalizableProcessor;
use crate::util::heap::gc_trigger::GCTrigger;
use crate::util::heap::layout::vm_layout::VMLayout;
use crate::util::heap::layout::{self, Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::opaque_pointer::*;
use crate::util::options::Options;
use crate::util::reference_processor::ReferenceProcessors;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::SanityChecker;
use crate::util::statistics::stats::Stats;
use crate::vm::ReferenceGlue;
use crate::vm::VMBinding;
use std::cell::UnsafeCell;
use std::default::Default;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

lazy_static! {
    // I am not sure if we should include these mmappers as part of MMTk struct.
    // The considerations are:
    // 1. We need VMMap and Mmapper to create spaces. It is natural that the mappers are not
    //    part of MMTK, as creating MMTK requires these mappers. We could use Rc/Arc for these mappers though.
    // 2. These mmappers are possibly global across multiple MMTk instances, as they manage the
    //    entire address space.
    // TODO: We should refactor this when we know more about how multiple MMTK instances work.

    /// A global VMMap that manages the mapping of spaces to virtual memory ranges.
    pub static ref VM_MAP: Box<dyn VMMap + Send + Sync> = layout::create_vm_map();

    /// A global Mmapper for mmaping and protection of virtual memory.
    pub static ref MMAPPER: Box<dyn Mmapper + Send + Sync> = layout::create_mmapper();
}

use crate::util::rust_util::InitializeOnce;

// A global space function table that allows efficient dispatch space specific code for addresses in our heap.
pub static SFT_MAP: InitializeOnce<Box<dyn SFTMap>> = InitializeOnce::new();

/// MMTk builder. This is used to set options and other settings before actually creating an MMTk instance.
pub struct MMTKBuilder {
    /// The options for this instance.
    pub options: Options,
}

impl MMTKBuilder {
    /// Create an MMTK builder with options read from environment variables, or using built-in
    /// default if not overridden by environment variables.
    pub fn new() -> Self {
        let mut builder = Self::new_no_env_vars();
        builder.options.read_env_var_settings();
        builder
    }

    /// Create an MMTK builder with build-in default options, but without reading options from
    /// environment variables.
    pub fn new_no_env_vars() -> Self {
        MMTKBuilder {
            options: Options::default(),
        }
    }

    /// Set an option.
    pub fn set_option(&mut self, name: &str, val: &str) -> bool {
        self.options.set_from_command_line(name, val)
    }

    /// Set multiple options by a string. The string should be key-value pairs separated by white spaces,
    /// such as `threads=1 stress_factor=4096`.
    pub fn set_options_bulk_by_str(&mut self, options: &str) -> bool {
        self.options.set_bulk_from_command_line(options)
    }

    /// Custom VM layout constants. VM bindings may use this function for compressed or 39-bit heap support.
    /// This function must be called before MMTk::new()
    pub fn set_vm_layout(&mut self, constants: VMLayout) {
        VMLayout::set_custom_vm_layout(constants)
    }

    /// Build an MMTk instance from the builder.
    pub fn build<VM: VMBinding>(&self) -> MMTK<VM> {
        MMTK::new(Arc::new(self.options.clone()))
    }
}

impl Default for MMTKBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// An MMTk instance. MMTk allows multiple instances to run independently, and each instance gives users a separate heap.
/// *Note that multi-instances is not fully supported yet*
pub struct MMTK<VM: VMBinding> {
    pub(crate) options: Arc<Options>,
    pub(crate) state: Arc<GlobalState>,
    pub(crate) plan: UnsafeCell<Box<dyn Plan<VM = VM>>>,
    pub(crate) reference_processors: ReferenceProcessors,
    pub(crate) finalizable_processor:
        Mutex<FinalizableProcessor<<VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType>>,
    pub(crate) scheduler: Arc<GCWorkScheduler<VM>>,
    #[cfg(feature = "sanity")]
    pub(crate) sanity_checker: Mutex<SanityChecker<VM::VMEdge>>,
    #[cfg(feature = "extreme_assertions")]
    pub(crate) edge_logger: EdgeLogger<VM::VMEdge>,
    pub(crate) gc_trigger: Arc<GCTrigger<VM>>,
    pub(crate) gc_requester: Arc<GCRequester<VM>>,
    pub(crate) stats: Arc<Stats>,
    inside_harness: AtomicBool,
    #[cfg(feature = "sanity")]
    inside_sanity: AtomicBool,
    /// Analysis counters. The feature analysis allows us to periodically stop the world and collect some statistics.
    #[cfg(feature = "analysis")]
    pub(crate) analysis_manager: Arc<AnalysisManager<VM>>,
}

unsafe impl<VM: VMBinding> Sync for MMTK<VM> {}
unsafe impl<VM: VMBinding> Send for MMTK<VM> {}

impl<VM: VMBinding> MMTK<VM> {
    /// Create an MMTK instance. This is not public. Bindings should use [`MMTKBuilder::build`].
    pub(crate) fn new(options: Arc<Options>) -> Self {
        // Initialize SFT first in case we need to use this in the constructor.
        // The first call will initialize SFT map. Other calls will be blocked until SFT map is initialized.
        crate::policy::sft_map::SFTRefStorage::pre_use_check();
        SFT_MAP.initialize_once(&create_sft_map);

        let num_workers = if cfg!(feature = "single_worker") {
            1
        } else {
            *options.threads
        };

        let scheduler = GCWorkScheduler::new(num_workers, (*options.thread_affinity).clone());

        let state = Arc::new(GlobalState::default());

        let gc_requester = Arc::new(GCRequester::new());

        let gc_trigger = Arc::new(GCTrigger::new(
            options.clone(),
            gc_requester.clone(),
            state.clone(),
        ));

        let stats = Arc::new(Stats::new(&options));

        // We need this during creating spaces, but we do not use this once the MMTk instance is created.
        // So we do not save it in MMTK. This may change in the future.
        let mut heap = HeapMeta::new();

        let plan = crate::plan::create_plan(
            *options.plan,
            CreateGeneralPlanArgs {
                vm_map: VM_MAP.as_ref(),
                mmapper: MMAPPER.as_ref(),
                options: options.clone(),
                state: state.clone(),
                gc_trigger: gc_trigger.clone(),
                scheduler: scheduler.clone(),
                stats: &stats,
                heap: &mut heap,
            },
        );

        // We haven't finished creating MMTk. No one is using the GC trigger. We cast the arc into a mutable reference.
        {
            // TODO: use Arc::get_mut_unchecked() when it is availble.
            let gc_trigger: &mut GCTrigger<VM> =
                unsafe { &mut *(Arc::as_ptr(&gc_trigger) as *mut _) };
            // We know the plan address will not change. Cast it to a static reference.
            let static_plan: &'static dyn Plan<VM = VM> = unsafe { &*(&*plan as *const _) };
            // Set the plan so we can trigger GC and check GC condition without using plan
            gc_trigger.set_plan(static_plan);
        }

        // TODO: This probably does not work if we have multiple MMTk instances.
        VM_MAP.boot();
        // This needs to be called after we create Plan. It needs to use HeapMeta, which is gradually built when we create spaces.
        VM_MAP.finalize_static_space_map(heap.get_discontig_start(), heap.get_discontig_end());

        if *options.transparent_hugepages {
            MMAPPER.set_mmap_strategy(crate::util::memory::MmapStrategy::TransparentHugePages);
        }

        MMTK {
            options,
            state,
            plan: UnsafeCell::new(plan),
            reference_processors: ReferenceProcessors::new(),
            finalizable_processor: Mutex::new(FinalizableProcessor::<
                <VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType,
            >::new()),
            scheduler,
            #[cfg(feature = "sanity")]
            sanity_checker: Mutex::new(SanityChecker::new()),
            #[cfg(feature = "sanity")]
            inside_sanity: AtomicBool::new(false),
            inside_harness: AtomicBool::new(false),
            #[cfg(feature = "extreme_assertions")]
            edge_logger: EdgeLogger::new(),
            #[cfg(feature = "analysis")]
            analysis_manager: Arc::new(AnalysisManager::new(stats.clone())),
            gc_trigger,
            gc_requester,
            stats,
        }
    }

    /// Generic hook to allow benchmarks to be harnessed. MMTk will trigger a GC
    /// to clear any residual garbage and start collecting statistics for the benchmark.
    /// This is usually called by the benchmark harness as its last step before the actual benchmark.
    pub fn harness_begin(&self, tls: VMMutatorThread) {
        probe!(mmtk, harness_begin);
        self.handle_user_collection_request(tls, true, true);
        self.inside_harness.store(true, Ordering::SeqCst);
        self.stats.start_all();
        self.scheduler.enable_stat();
    }

    /// Generic hook to allow benchmarks to be harnessed. MMTk will stop collecting
    /// statistics, and print out the collected statistics in a defined format.
    /// This is usually called by the benchmark harness right after the actual benchmark.
    pub fn harness_end(&'static self) {
        self.stats.stop_all(self);
        self.inside_harness.store(false, Ordering::SeqCst);
        probe!(mmtk, harness_end);
    }

    #[cfg(feature = "sanity")]
    pub(crate) fn sanity_begin(&self) {
        self.inside_sanity.store(true, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    pub(crate) fn sanity_end(&self) {
        self.inside_sanity.store(false, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    pub(crate) fn is_in_sanity(&self) -> bool {
        self.inside_sanity.load(Ordering::Relaxed)
    }

    pub(crate) fn set_gc_status(&self, s: GcStatus) {
        let mut gc_status = self.state.gc_status.lock().unwrap();
        if *gc_status == GcStatus::NotInGC {
            self.state.stacks_prepared.store(false, Ordering::SeqCst);
            // FIXME stats
            self.stats.start_gc();
        }
        *gc_status = s;
        if *gc_status == GcStatus::NotInGC {
            // FIXME stats
            if self.stats.get_gathering_stats() {
                self.stats.end_gc();
            }
        }
    }

    /// Return true if a collection is in progress.
    pub fn gc_in_progress(&self) -> bool {
        *self.state.gc_status.lock().unwrap() != GcStatus::NotInGC
    }

    /// Return true if a collection is in progress and past the preparatory stage.
    pub fn gc_in_progress_proper(&self) -> bool {
        *self.state.gc_status.lock().unwrap() == GcStatus::GcProper
    }

    /// Return true if the current GC is an emergency GC.
    ///
    /// An emergency GC happens when a normal GC cannot reclaim enough memory to satisfy allocation
    /// requests.  Plans may do full-heap GC, defragmentation, etc. during emergency in order to
    /// free up more memory.
    ///
    /// VM bindings can call this function during GC to check if the current GC is an emergency GC.
    /// If it is, the VM binding is recommended to retain fewer objects than normal GCs, to the
    /// extent allowed by the specification of the VM or langauge.  For example, the VM binding may
    /// choose not to retain objects used for caching.  Specifically, for Java virtual machines,
    /// that means not retaining referents of [`SoftReference`][java-soft-ref] which is primarily
    /// designed for implementing memory-sensitive caches.
    ///
    /// [java-soft-ref]: https://docs.oracle.com/en/java/javase/21/docs/api/java.base/java/lang/ref/SoftReference.html
    pub fn is_emergency_collection(&self) -> bool {
        self.state.is_emergency_collection()
    }

    /// The application code has requested a collection. This is just a GC hint, and
    /// we may ignore it.
    ///
    /// # Arguments
    /// * `tls`: The mutator thread that requests the GC
    /// * `force`: The request cannot be ignored (except for NoGC)
    /// * `exhaustive`: The requested GC should be exhaustive. This is also a hint.
    pub fn handle_user_collection_request(
        &self,
        tls: VMMutatorThread,
        force: bool,
        exhaustive: bool,
    ) {
        use crate::vm::Collection;
        if !self.get_plan().constraints().collects_garbage {
            warn!("User attempted a collection request, but the plan can not do GC. The request is ignored.");
            return;
        }

        if force || !*self.options.ignore_system_gc && !VM::VMCollection::is_collection_disabled() {
            info!("User triggering collection");
            if exhaustive {
                if let Some(gen) = self.get_plan().generational() {
                    gen.force_full_heap_collection();
                }
            }

            self.state
                .user_triggered_collection
                .store(true, Ordering::Relaxed);
            self.gc_requester.request();
            VM::VMCollection::block_for_gc(tls);
        }
    }

    /// MMTK has requested stop-the-world activity (e.g., stw within a concurrent gc).
    // This is not used, as we do not have a concurrent plan.
    #[allow(unused)]
    pub fn trigger_internal_collection_request(&self) {
        self.state
            .last_internal_triggered_collection
            .store(true, Ordering::Relaxed);
        self.state
            .internal_triggered_collection
            .store(true, Ordering::Relaxed);
        self.gc_requester.request();
    }

    /// Get a reference to the plan.
    pub fn get_plan(&self) -> &dyn Plan<VM = VM> {
        unsafe { &**(self.plan.get()) }
    }

    /// Get the plan as mutable reference.
    ///
    /// # Safety
    ///
    /// This is unsafe because the caller must ensure that the plan is not used by other threads.
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn get_plan_mut(&self) -> &mut dyn Plan<VM = VM> {
        &mut **(self.plan.get())
    }

    /// Get the run time options.
    pub fn get_options(&self) -> &Options {
        &self.options
    }
}
