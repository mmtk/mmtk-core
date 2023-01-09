//! The global part of a plan implementation.

use super::gc_requester::GCRequester;
use super::PlanConstraints;
use crate::mmtk::MMTK;
use crate::plan::generational::global::Gen;
use crate::plan::tracing::ObjectQueue;
use crate::plan::Mutator;
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::{PlanCreateSpaceArgs, Space};
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(feature = "analysis")]
use crate::util::analysis::AnalysisManager;
use crate::util::copy::{CopyConfig, GCWorkerCopyContext};
use crate::util::heap::gc_trigger::GCTrigger;
use crate::util::heap::layout::heap_layout::Map;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::options::Options;
use crate::util::options::PlanSelector;
use crate::util::statistics::stats::Stats;
use crate::util::ObjectReference;
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::*;
use downcast_rs::Downcast;
use enum_map::EnumMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mmtk_macros::PlanTraceObject;

pub fn create_mutator<VM: VMBinding>(
    tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Box<Mutator<VM>> {
    Box::new(match *mmtk.options.plan {
        PlanSelector::NoGC => crate::plan::nogc::mutator::create_nogc_mutator(tls, &*mmtk.plan),
        PlanSelector::SemiSpace => {
            crate::plan::semispace::mutator::create_ss_mutator(tls, &*mmtk.plan)
        }
        PlanSelector::GenCopy => {
            crate::plan::generational::copying::mutator::create_gencopy_mutator(tls, mmtk)
        }
        PlanSelector::GenImmix => {
            crate::plan::generational::immix::mutator::create_genimmix_mutator(tls, mmtk)
        }
        PlanSelector::MarkSweep => {
            crate::plan::marksweep::mutator::create_ms_mutator(tls, &*mmtk.plan)
        }
        PlanSelector::Immix => crate::plan::immix::mutator::create_immix_mutator(tls, &*mmtk.plan),
        PlanSelector::PageProtect => {
            crate::plan::pageprotect::mutator::create_pp_mutator(tls, &*mmtk.plan)
        }
        PlanSelector::MarkCompact => {
            crate::plan::markcompact::mutator::create_markcompact_mutator(tls, &*mmtk.plan)
        }
    })
}

pub fn create_plan<VM: VMBinding>(
    plan: PlanSelector,
    vm_map: &'static dyn Map,
    mmapper: &'static dyn Mmapper,
    options: Arc<Options>,
    scheduler: Arc<GCWorkScheduler<VM>>,
) -> Box<dyn Plan<VM = VM>> {
    let args = CreateGeneralPlanArgs {
        vm_map,
        mmapper,
        heap: HeapMeta::new(),
        gc_trigger: Arc::new(GCTrigger::new(&options)),
        options,
        scheduler,
    };
    let gc_trigger = args.gc_trigger.clone();

    let plan = match plan {
        PlanSelector::NoGC => {
            Box::new(crate::plan::nogc::NoGC::new(args)) as Box<dyn Plan<VM = VM>>
        }
        PlanSelector::SemiSpace => {
            Box::new(crate::plan::semispace::SemiSpace::new(args)) as Box<dyn Plan<VM = VM>>
        }
        PlanSelector::GenCopy => Box::new(crate::plan::generational::copying::GenCopy::new(args))
            as Box<dyn Plan<VM = VM>>,
        PlanSelector::GenImmix => Box::new(crate::plan::generational::immix::GenImmix::new(args))
            as Box<dyn Plan<VM = VM>>,
        PlanSelector::MarkSweep => {
            Box::new(crate::plan::marksweep::MarkSweep::new(args)) as Box<dyn Plan<VM = VM>>
        }
        PlanSelector::Immix => {
            Box::new(crate::plan::immix::Immix::new(args)) as Box<dyn Plan<VM = VM>>
        }
        PlanSelector::PageProtect => {
            Box::new(crate::plan::pageprotect::PageProtect::new(args)) as Box<dyn Plan<VM = VM>>
        }
        PlanSelector::MarkCompact => {
            Box::new(crate::plan::markcompact::MarkCompact::new(args)) as Box<dyn Plan<VM = VM>>
        }
    };

    // We have created Plan in the heap, and we won't explicitly move it.

    // The plan has a fixed address. Set plan in gc_trigger
    {
        // We haven't finished creating the plan. No one is using the GC trigger. We cast the arc into a mutable reference.
        // TODO: use Arc::get_mut_unchecked() when it is availble.
        let gc_trigger: &mut GCTrigger<VM> = unsafe { &mut *(Arc::as_ptr(&gc_trigger) as *mut _) };
        // We know the plan address will not change. Cast it to a static reference.
        let static_plan: &'static dyn Plan<VM = VM> = unsafe { &*(&*plan as *const _) };
        // Set the plan so we can trigger GC and check GC condition without using plan
        gc_trigger.set_plan(static_plan);
    }

    // Each space now has a fixed address for its lifetime. It is safe now to initialize SFT.
    plan.get_spaces()
        .into_iter()
        .for_each(|s| s.initialize_sft());

    plan
}

/// Create thread local GC worker.
pub fn create_gc_worker_context<VM: VMBinding>(
    tls: VMWorkerThread,
    mmtk: &'static MMTK<VM>,
) -> GCWorkerCopyContext<VM> {
    GCWorkerCopyContext::<VM>::new(tls, &*mmtk.plan, mmtk.plan.create_copy_config())
}

/// A plan describes the global core functionality for all memory management schemes.
/// All global MMTk plans should implement this trait.
///
/// The global instance defines and manages static resources
/// (such as memory and virtual memory resources).
///
/// Constructor:
///
/// For the constructor of a new plan, there are a few things the constructor _must_ do
/// (please check existing plans and see what they do in the constructor):
/// 1. Create a HeapMeta, and use this HeapMeta to initialize all the spaces.
/// 2. Create a vector of all the side metadata specs with `SideMetadataContext::new_global_specs()`,
///    the parameter is a vector of global side metadata specs that are specific to the plan.
/// 3. Initialize all the spaces the plan uses with the heap meta, and the global metadata specs vector.
/// 4. Create a `SideMetadataSanity` object, and invoke verify_side_metadata_sanity() for each space (or
///    invoke verify_side_metadata_sanity() in `CommonPlan`/`BasePlan` for the spaces in the common/base plan).
///
/// Methods in this trait:
///
/// Only methods that will be overridden by each specific plan should be included in this trait. The trait may
/// provide a default implementation, and each plan can override the implementation. For methods that won't be
/// overridden, we should implement those methods in BasePlan (or CommonPlan) and call them from there instead.
/// We should avoid having methods with the same name in both Plan and BasePlan, as this may confuse people, and
/// they may call a wrong method by mistake.
// TODO: Some methods that are not overriden can be moved from the trait to BasePlan.
pub trait Plan: 'static + Sync + Downcast {
    type VM: VMBinding;

    fn constraints(&self) -> &'static PlanConstraints;

    /// Create a copy config for this plan. A copying GC plan MUST override this method,
    /// and provide a valid config.
    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        // Use the empty default copy config for non copying GC.
        CopyConfig::default()
    }

    fn base(&self) -> &BasePlan<Self::VM>;
    fn schedule_collection(&'static self, _scheduler: &GCWorkScheduler<Self::VM>);
    fn common(&self) -> &CommonPlan<Self::VM> {
        panic!("Common Plan not handled!")
    }
    fn generational(&self) -> &Gen<Self::VM> {
        panic!("This is not a generational plan.")
    }
    fn mmapper(&self) -> &'static dyn Mmapper {
        self.base().mmapper
    }
    fn options(&self) -> &Options {
        &self.base().options
    }

    /// get all the spaces in the plan
    fn get_spaces(&self) -> Vec<&dyn Space<Self::VM>>;

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector>;

    /// Is current GC only collecting objects allocated since last GC?
    fn is_current_gc_nursery(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn enter_sanity(&self) {
        self.base().inside_sanity.store(true, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    fn leave_sanity(&self) {
        self.base().inside_sanity.store(false, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    fn is_in_sanity(&self) -> bool {
        self.base().inside_sanity.load(Ordering::Relaxed)
    }

    fn is_initialized(&self) -> bool {
        self.base().initialized.load(Ordering::SeqCst)
    }

    fn should_trigger_gc_when_heap_is_full(&self) -> bool {
        self.base()
            .trigger_gc_when_heap_is_full
            .load(Ordering::SeqCst)
    }

    /// Prepare the plan before a GC. This is invoked in an initial step in the GC.
    /// This is invoked once per GC by one worker thread. `tls` is the worker thread that executes this method.
    fn prepare(&mut self, tls: VMWorkerThread);

    /// Prepare a worker for a GC. Each worker has its own prepare method. This hook is for plan-specific
    /// per-worker preparation. This method is invoked once per worker by the worker thread passed as the argument.
    fn prepare_worker(&self, _worker: &mut GCWorker<Self::VM>) {}

    /// Release the plan after transitive closure. A plan can implement this method to call each policy's release,
    /// or create any work packet that should be done in release.
    /// This is invoked once per GC by one worker thread. `tls` is the worker thread that executes this method.
    fn release(&mut self, tls: VMWorkerThread);

    /// Inform the plan about the end of a GC. It is guaranteed that there is no further work for this GC.
    /// This is invoked once per GC by one worker thread. `tls` is the worker thread that executes this method.
    fn end_of_gc(&mut self, _tls: VMWorkerThread) {}

    /// Ask the plan if they would trigger a GC. If MMTk is in charge of triggering GCs, this method is called
    /// periodically during allocation. However, MMTk may delegate the GC triggering decision to the runtime,
    /// in which case, this method may not be called. This method returns true to trigger a collection.
    ///
    /// # Arguments
    /// * `space_full`: the allocation to a specific space failed, must recover pages within 'space'.
    /// * `space`: an option to indicate if there is a space that has failed in an allocation.
    fn collection_required(&self, space_full: bool, space: Option<&dyn Space<Self::VM>>) -> bool;

    // Note: The following methods are about page accounting. The default implementation should
    // work fine for non-copying plans. For copying plans, the plan should override any of these methods
    // if necessary.

    /// Get the number of pages that are reserved, including used pages and pages that will
    /// be used (e.g. for copying).
    fn get_reserved_pages(&self) -> usize {
        self.get_used_pages() + self.get_collection_reserved_pages()
    }

    /// Get the total number of pages for the heap.
    fn get_total_pages(&self) -> usize {
        self.base().gc_trigger.policy.get_heap_size_in_pages()
    }

    /// Get the number of pages that are still available for use. The available pages
    /// should always be positive or 0.
    fn get_available_pages(&self) -> usize {
        // It is possible that the reserved pages is larger than the total pages so we are doing
        // a saturating subtraction to make sure we return a non-negative number.
        // For example,
        // 1. our GC trigger checks if reserved pages is more than total pages.
        // 2. when the heap is almost full of live objects (such as in the case of an OOM) and we are doing a copying GC, it is possible
        //    the reserved pages is larger than total pages after the copying GC (the reserved pages after a GC
        //    may be larger than the reserved pages before a GC, as we may end up using more memory for thread local
        //    buffers for copy allocators).
        self.get_total_pages()
            .saturating_sub(self.get_reserved_pages())
    }

    /// Return the number of pages available for allocation into the mature space. Only
    /// generational plans have to implement this function.
    fn get_mature_physical_pages_available(&self) -> usize {
        panic!("This is not a generational plan.")
    }

    /// Get the number of pages that are reserved for collection. By default, we return 0.
    /// For copying plans, they need to override this and calculate required pages to complete
    /// a copying GC.
    fn get_collection_reserved_pages(&self) -> usize {
        0
    }

    /// Get the number of pages that are used.
    fn get_used_pages(&self) -> usize;

    /// Get the number of pages that are NOT used. This is clearly different from available pages.
    /// Free pages are unused, but some of them may have been reserved for some reason.
    fn get_free_pages(&self) -> usize {
        self.get_total_pages() - self.get_used_pages()
    }

    fn is_emergency_collection(&self) -> bool {
        self.base().emergency_collection.load(Ordering::Relaxed)
    }

    /// The application code has requested a collection.
    fn handle_user_collection_request(&self, tls: VMMutatorThread, force: bool) {
        self.base().handle_user_collection_request(tls, force)
    }

    /// Return whether last GC was an exhaustive attempt to collect the heap.
    /// For many collectors this is the same as asking whether the last GC was a full heap collection.
    fn last_collection_was_exhaustive(&self) -> bool {
        self.last_collection_full_heap()
    }

    /// Return whether last GC is a full GC.
    fn last_collection_full_heap(&self) -> bool {
        true
    }

    /// Force the next collection to be full heap.
    fn force_full_heap_collection(&self) {}

    fn modify_check(&self, object: ObjectReference) {
        assert!(
            !(self.base().gc_in_progress_proper() && object.is_movable()),
            "GC modifying a potentially moving object via Java (i.e. not magic) obj= {}",
            object
        );
    }
}

impl_downcast!(Plan assoc VM);

#[derive(PartialEq)]
pub enum GcStatus {
    NotInGC,
    GcPrepare,
    GcProper,
}

/**
BasePlan should contain all plan-related state and functions that are _fundamental_ to _all_ plans.  These include VM-specific (but not plan-specific) features such as a code space or vm space, which are fundamental to all plans for a given VM.  Features that are common to _many_ (but not intrinsically _all_) plans should instead be included in CommonPlan.
*/
#[derive(PlanTraceObject)]
pub struct BasePlan<VM: VMBinding> {
    /// Whether MMTk is now ready for collection. This is set to true when initialize_collection() is called.
    pub initialized: AtomicBool,
    /// Should we trigger a GC when the heap is full? It seems this should always be true. However, we allow
    /// bindings to temporarily disable GC, at which point, we do not trigger GC even if the heap is full.
    pub trigger_gc_when_heap_is_full: AtomicBool,
    pub gc_status: Mutex<GcStatus>,
    pub last_stress_pages: AtomicUsize,
    pub emergency_collection: AtomicBool,
    pub user_triggered_collection: AtomicBool,
    pub internal_triggered_collection: AtomicBool,
    pub last_internal_triggered_collection: AtomicBool,
    // Has an allocation succeeded since the emergency collection?
    pub allocation_success: AtomicBool,
    // Maximum number of failed attempts by a single thread
    pub max_collection_attempts: AtomicUsize,
    // Current collection attempt
    pub cur_collection_attempts: AtomicUsize,
    pub gc_requester: Arc<GCRequester<VM>>,
    pub stats: Stats,
    mmapper: &'static dyn Mmapper,
    pub vm_map: &'static dyn Map,
    pub options: Arc<Options>,
    pub heap: HeapMeta,
    pub gc_trigger: Arc<GCTrigger<VM>>,
    #[cfg(feature = "sanity")]
    pub inside_sanity: AtomicBool,
    /// A counter for per-mutator stack scanning
    scanned_stacks: AtomicUsize,
    /// Have we scanned all the stacks?
    stacks_prepared: AtomicBool,
    pub mutator_iterator_lock: Mutex<()>,
    /// A counter that keeps tracks of the number of bytes allocated since last stress test
    allocation_bytes: AtomicUsize,
    /// A counteer that keeps tracks of the number of bytes allocated by malloc
    #[cfg(feature = "malloc_counted_size")]
    malloc_bytes: AtomicUsize,
    /// Wrapper around analysis counters
    #[cfg(feature = "analysis")]
    pub analysis_manager: AnalysisManager<VM>,

    // Spaces in base plan
    #[cfg(feature = "code_space")]
    #[trace]
    pub code_space: ImmortalSpace<VM>,
    #[cfg(feature = "code_space")]
    #[trace]
    pub code_lo_space: ImmortalSpace<VM>,
    #[cfg(feature = "ro_space")]
    #[trace]
    pub ro_space: ImmortalSpace<VM>,

    /// A VM space is a space allocated and populated by the VM.  Currently it is used by JikesRVM
    /// for boot image.
    ///
    /// If VM space is present, it has some special interaction with the
    /// `memory_manager::is_mmtk_object` and the `memory_manager::is_in_mmtk_spaces` functions.
    ///
    /// -   The `is_mmtk_object` funciton requires the alloc_bit side metadata to identify objects,
    ///     but currently we do not require the boot image to provide it, so it will not work if the
    ///     address argument is in the VM space.
    ///
    /// -   The `is_in_mmtk_spaces` currently returns `true` if the given object reference is in
    ///     the VM space.
    #[cfg(feature = "vm_space")]
    #[trace]
    pub vm_space: ImmortalSpace<VM>,
}

#[cfg(feature = "vm_space")]
pub fn create_vm_space<VM: VMBinding>(args: &mut CreateSpecificPlanArgs<VM>) -> ImmortalSpace<VM> {
    use crate::util::constants::LOG_BYTES_IN_MBYTE;
    let boot_segment_bytes = *args.global_args.options.vm_space_size;
    debug_assert!(boot_segment_bytes > 0);

    use crate::util::conversions::raw_align_up;
    use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
    let boot_segment_mb = raw_align_up(boot_segment_bytes, BYTES_IN_CHUNK) >> LOG_BYTES_IN_MBYTE;

    let space = ImmortalSpace::new(args.get_space_args(
        "boot",
        false,
        VMRequest::fixed_size(boot_segment_mb),
    ));

    // The space is mapped externally by the VM. We need to update our mmapper to mark the range as mapped.
    space.ensure_mapped();

    space
}

/// Args needed for creating any plan. This includes a set of contexts from MMTK or global. This
/// is passed to each plan's constructor.
pub struct CreateGeneralPlanArgs<VM: VMBinding> {
    pub vm_map: &'static dyn Map,
    pub mmapper: &'static dyn Mmapper,
    pub heap: HeapMeta,
    pub options: Arc<Options>,
    pub gc_trigger: Arc<crate::util::heap::gc_trigger::GCTrigger<VM>>,
    pub scheduler: Arc<GCWorkScheduler<VM>>,
}

/// Args needed for creating a specific plan. This includes plan-specific args, such as plan constrainst
/// and their global side metadata specs. This is created in each plan's constructor, and will be passed
/// to `CommonPlan` or `BasePlan`. Also you can create `PlanCreateSpaceArg` from this type, and use that
/// to create spaces.
pub struct CreateSpecificPlanArgs<VM: VMBinding> {
    pub global_args: CreateGeneralPlanArgs<VM>,
    pub constraints: &'static PlanConstraints,
    pub global_side_metadata_specs: Vec<SideMetadataSpec>,
}

impl<VM: VMBinding> CreateSpecificPlanArgs<VM> {
    /// Get a PlanCreateSpaceArgs that can be used to create a space
    pub fn get_space_args(
        &mut self,
        name: &'static str,
        zeroed: bool,
        vmrequest: VMRequest,
    ) -> PlanCreateSpaceArgs<VM> {
        PlanCreateSpaceArgs {
            name,
            zeroed,
            vmrequest,
            global_side_metadata_specs: self.global_side_metadata_specs.clone(),
            vm_map: self.global_args.vm_map,
            mmapper: self.global_args.mmapper,
            heap: &mut self.global_args.heap,
            constraints: self.constraints,
            gc_trigger: self.global_args.gc_trigger.clone(),
            scheduler: self.global_args.scheduler.clone(),
            options: &self.global_args.options,
        }
    }
}

impl<VM: VMBinding> BasePlan<VM> {
    #[allow(unused_mut)] // 'args' only needs to be mutable for certain features
    pub fn new(mut args: CreateSpecificPlanArgs<VM>) -> BasePlan<VM> {
        let stats = Stats::new(&args.global_args.options);
        // Initializing the analysis manager and routines
        #[cfg(feature = "analysis")]
        let analysis_manager = AnalysisManager::new(&stats);
        BasePlan {
            #[cfg(feature = "code_space")]
            code_space: ImmortalSpace::new(args.get_space_args(
                "code_space",
                true,
                VMRequest::discontiguous(),
            )),
            #[cfg(feature = "code_space")]
            code_lo_space: ImmortalSpace::new(args.get_space_args(
                "code_lo_space",
                true,
                VMRequest::discontiguous(),
            )),
            #[cfg(feature = "ro_space")]
            ro_space: ImmortalSpace::new(args.get_space_args(
                "ro_space",
                true,
                VMRequest::discontiguous(),
            )),
            #[cfg(feature = "vm_space")]
            vm_space: create_vm_space(&mut args),

            initialized: AtomicBool::new(false),
            trigger_gc_when_heap_is_full: AtomicBool::new(true),
            gc_status: Mutex::new(GcStatus::NotInGC),
            last_stress_pages: AtomicUsize::new(0),
            stacks_prepared: AtomicBool::new(false),
            emergency_collection: AtomicBool::new(false),
            user_triggered_collection: AtomicBool::new(false),
            internal_triggered_collection: AtomicBool::new(false),
            last_internal_triggered_collection: AtomicBool::new(false),
            allocation_success: AtomicBool::new(false),
            max_collection_attempts: AtomicUsize::new(0),
            cur_collection_attempts: AtomicUsize::new(0),
            gc_requester: Arc::new(GCRequester::new()),
            stats,
            mmapper: args.global_args.mmapper,
            heap: args.global_args.heap,
            gc_trigger: args.global_args.gc_trigger,
            vm_map: args.global_args.vm_map,
            options: args.global_args.options.clone(),
            #[cfg(feature = "sanity")]
            inside_sanity: AtomicBool::new(false),
            scanned_stacks: AtomicUsize::new(0),
            mutator_iterator_lock: Mutex::new(()),
            allocation_bytes: AtomicUsize::new(0),
            #[cfg(feature = "malloc_counted_size")]
            malloc_bytes: AtomicUsize::new(0),
            #[cfg(feature = "analysis")]
            analysis_manager,
        }
    }

    pub fn get_spaces(&self) -> Vec<&dyn Space<VM>> {
        vec![
            #[cfg(feature = "code_space")]
            &self.code_space,
            #[cfg(feature = "code_space")]
            &self.code_lo_space,
            #[cfg(feature = "ro_space")]
            &self.ro_space,
            #[cfg(feature = "vm_space")]
            &self.vm_space,
        ]
    }

    /// The application code has requested a collection.
    pub fn handle_user_collection_request(&self, tls: VMMutatorThread, force: bool) {
        if force || !*self.options.ignore_system_gc {
            info!("User triggering collection");
            self.user_triggered_collection
                .store(true, Ordering::Relaxed);
            self.gc_requester.request();
            VM::VMCollection::block_for_gc(tls);
        }
    }

    /// MMTK has requested stop-the-world activity (e.g., stw within a concurrent gc).
    // This is not used, as we do not have a concurrent plan.
    #[allow(unused)]
    pub fn trigger_internal_collection_request(&self) {
        self.last_internal_triggered_collection
            .store(true, Ordering::Relaxed);
        self.internal_triggered_collection
            .store(true, Ordering::Relaxed);
        self.gc_requester.request();
    }

    /// Reset collection state information.
    pub fn reset_collection_trigger(&self) {
        self.last_internal_triggered_collection.store(
            self.internal_triggered_collection.load(Ordering::SeqCst),
            Ordering::Relaxed,
        );
        self.internal_triggered_collection
            .store(false, Ordering::SeqCst);
        self.user_triggered_collection
            .store(false, Ordering::Relaxed);
    }

    // Depends on what base spaces we use, unsync may be unused.
    pub fn get_used_pages(&self) -> usize {
        // Depends on what base spaces we use, pages may be unchanged.
        #[allow(unused_mut)]
        let mut pages = 0;

        #[cfg(feature = "code_space")]
        {
            pages += self.code_space.reserved_pages();
            pages += self.code_lo_space.reserved_pages();
        }
        #[cfg(feature = "ro_space")]
        {
            pages += self.ro_space.reserved_pages();
        }

        // If we need to count malloc'd size as part of our heap, we add it here.
        #[cfg(feature = "malloc_counted_size")]
        {
            pages += crate::util::conversions::bytes_to_pages_up(
                self.malloc_bytes.load(Ordering::SeqCst),
            );
        }

        // The VM space may be used as an immutable boot image, in which case, we should not count
        // it as part of the heap size.
        pages
    }

    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        #[cfg(feature = "code_space")]
        if self.code_space.in_space(object) {
            trace!("trace_object: object in code space");
            return self.code_space.trace_object::<Q>(queue, object);
        }

        #[cfg(feature = "code_space")]
        if self.code_lo_space.in_space(object) {
            trace!("trace_object: object in large code space");
            return self.code_lo_space.trace_object::<Q>(queue, object);
        }

        #[cfg(feature = "ro_space")]
        if self.ro_space.in_space(object) {
            trace!("trace_object: object in ro_space space");
            return self.ro_space.trace_object(queue, object);
        }

        #[cfg(feature = "vm_space")]
        if self.vm_space.in_space(object) {
            trace!("trace_object: object in boot space");
            return self.vm_space.trace_object(queue, object);
        }

        VM::VMActivePlan::vm_trace_object::<Q>(queue, object, worker)
    }

    pub fn prepare(&mut self, _tls: VMWorkerThread, _full_heap: bool) {
        #[cfg(feature = "code_space")]
        self.code_space.prepare();
        #[cfg(feature = "code_space")]
        self.code_lo_space.prepare();
        #[cfg(feature = "ro_space")]
        self.ro_space.prepare();
        #[cfg(feature = "vm_space")]
        self.vm_space.prepare();
    }

    pub fn release(&mut self, _tls: VMWorkerThread, _full_heap: bool) {
        #[cfg(feature = "code_space")]
        self.code_space.release();
        #[cfg(feature = "code_space")]
        self.code_lo_space.release();
        #[cfg(feature = "ro_space")]
        self.ro_space.release();
        #[cfg(feature = "vm_space")]
        self.vm_space.release();
    }

    pub fn set_collection_kind<P: Plan>(&self, plan: &P) {
        self.cur_collection_attempts.store(
            if self.is_user_triggered_collection() {
                1
            } else {
                self.determine_collection_attempts()
            },
            Ordering::Relaxed,
        );

        let emergency_collection = !self.is_internal_triggered_collection()
            && plan.last_collection_was_exhaustive()
            && self.cur_collection_attempts.load(Ordering::Relaxed) > 1
            && !self.gc_trigger.policy.can_heap_size_grow();
        self.emergency_collection
            .store(emergency_collection, Ordering::Relaxed);

        if emergency_collection {
            plan.force_full_heap_collection();
        }
    }

    pub fn set_gc_status(&self, s: GcStatus) {
        let mut gc_status = self.gc_status.lock().unwrap();
        if *gc_status == GcStatus::NotInGC {
            self.stacks_prepared.store(false, Ordering::SeqCst);
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

    /// Are the stacks scanned?
    pub fn stacks_prepared(&self) -> bool {
        self.stacks_prepared.load(Ordering::SeqCst)
    }

    /// Prepare for stack scanning. This is usually used with `inform_stack_scanned()`.
    /// This should be called before doing stack scanning.
    pub fn prepare_for_stack_scanning(&self) {
        self.scanned_stacks.store(0, Ordering::SeqCst);
        self.stacks_prepared.store(false, Ordering::SeqCst);
    }

    /// Inform that 1 stack has been scanned. The argument `n_mutators` indicates the
    /// total stacks we should scan. This method returns true if the number of scanned
    /// stacks equals the total mutator count. Otherwise it returns false. This method
    /// is thread safe and we guarantee only one thread will return true.
    pub fn inform_stack_scanned(&self, n_mutators: usize) -> bool {
        let old = self.scanned_stacks.fetch_add(1, Ordering::SeqCst);
        debug_assert!(
            old < n_mutators,
            "The number of scanned stacks ({}) is more than the number of mutators ({})",
            old,
            n_mutators
        );
        let scanning_done = old + 1 == n_mutators;
        if scanning_done {
            self.stacks_prepared.store(true, Ordering::SeqCst);
        }
        scanning_done
    }

    pub fn gc_in_progress(&self) -> bool {
        *self.gc_status.lock().unwrap() != GcStatus::NotInGC
    }

    pub fn gc_in_progress_proper(&self) -> bool {
        *self.gc_status.lock().unwrap() == GcStatus::GcProper
    }

    fn determine_collection_attempts(&self) -> usize {
        if !self.allocation_success.load(Ordering::Relaxed) {
            self.max_collection_attempts.fetch_add(1, Ordering::Relaxed);
        } else {
            self.allocation_success.store(false, Ordering::Relaxed);
            self.max_collection_attempts.store(1, Ordering::Relaxed);
        }

        self.max_collection_attempts.load(Ordering::Relaxed)
    }

    /// Return true if this collection was triggered by application code.
    pub fn is_user_triggered_collection(&self) -> bool {
        self.user_triggered_collection.load(Ordering::Relaxed)
    }

    /// Return true if this collection was triggered internally.
    pub fn is_internal_triggered_collection(&self) -> bool {
        let is_internal_triggered = self
            .last_internal_triggered_collection
            .load(Ordering::SeqCst);
        // Remove this assertion when we have concurrent GC.
        assert!(
            !is_internal_triggered,
            "We have no concurrent GC implemented. We should not have internally triggered GC"
        );
        is_internal_triggered
    }

    /// Increase the allocation bytes and return the current allocation bytes after increasing
    pub fn increase_allocation_bytes_by(&self, size: usize) -> usize {
        let old_allocation_bytes = self.allocation_bytes.fetch_add(size, Ordering::SeqCst);
        trace!(
            "Stress GC: old_allocation_bytes = {}, size = {}, allocation_bytes = {}",
            old_allocation_bytes,
            size,
            self.allocation_bytes.load(Ordering::Relaxed),
        );
        old_allocation_bytes + size
    }

    /// Check if the options are set for stress GC. If either stress_factor or analysis_factor is set,
    /// we should do stress GC.
    pub fn is_stress_test_gc_enabled(&self) -> bool {
        use crate::util::constants::DEFAULT_STRESS_FACTOR;
        *self.options.stress_factor != DEFAULT_STRESS_FACTOR
            || *self.options.analysis_factor != DEFAULT_STRESS_FACTOR
    }

    /// Check if we should do precise stress test. If so, we need to check for stress GCs for every allocation.
    /// Otherwise, we only check in the allocation slow path.
    pub fn is_precise_stress(&self) -> bool {
        *self.options.precise_stress
    }

    /// Check if we should do a stress GC now. If GC is initialized and the allocation bytes exceeds
    /// the stress factor, we should do a stress GC.
    pub fn should_do_stress_gc(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
            && (self.allocation_bytes.load(Ordering::SeqCst) > *self.options.stress_factor)
    }

    pub(super) fn collection_required<P: Plan>(&self, plan: &P, space_full: bool) -> bool {
        let stress_force_gc = self.should_do_stress_gc();
        if stress_force_gc {
            debug!(
                "Stress GC: allocation_bytes = {}, stress_factor = {}",
                self.allocation_bytes.load(Ordering::Relaxed),
                *self.options.stress_factor
            );
            debug!("Doing stress GC");
            self.allocation_bytes.store(0, Ordering::SeqCst);
        }

        debug!(
            "self.get_reserved_pages()={}, self.get_total_pages()={}",
            plan.get_reserved_pages(),
            plan.get_total_pages()
        );
        // Check if we reserved more pages (including the collection copy reserve)
        // than the heap's total pages. In that case, we will have to do a GC.
        let heap_full = plan.base().gc_trigger.is_heap_full();

        space_full || stress_force_gc || heap_full
    }

    #[allow(unused_variables)] // depending on the enabled features, base may not be used.
    pub(crate) fn verify_side_metadata_sanity(
        &self,
        side_metadata_sanity_checker: &mut SideMetadataSanity,
    ) {
        #[cfg(feature = "code_space")]
        self.code_space
            .verify_side_metadata_sanity(side_metadata_sanity_checker);
        #[cfg(feature = "ro_space")]
        self.ro_space
            .verify_side_metadata_sanity(side_metadata_sanity_checker);
        #[cfg(feature = "vm_space")]
        self.vm_space
            .verify_side_metadata_sanity(side_metadata_sanity_checker);
    }

    #[cfg(feature = "malloc_counted_size")]
    pub(crate) fn increase_malloc_bytes_by(&self, size: usize) {
        self.malloc_bytes.fetch_add(size, Ordering::SeqCst);
    }
    #[cfg(feature = "malloc_counted_size")]
    pub(crate) fn decrease_malloc_bytes_by(&self, size: usize) {
        self.malloc_bytes.fetch_sub(size, Ordering::SeqCst);
    }
    #[cfg(feature = "malloc_counted_size")]
    pub fn get_malloc_bytes(&self) -> usize {
        self.malloc_bytes.load(Ordering::SeqCst)
    }
}

/**
CommonPlan is for representing state and features used by _many_ plans, but that are not fundamental to _all_ plans.  Examples include the Large Object Space and an Immortal space.  Features that are fundamental to _all_ plans must be included in BasePlan.
*/
#[derive(PlanTraceObject)]
pub struct CommonPlan<VM: VMBinding> {
    #[trace]
    pub immortal: ImmortalSpace<VM>,
    #[trace]
    pub los: LargeObjectSpace<VM>,
    // TODO: We should use a marksweep space for nonmoving.
    #[trace]
    pub nonmoving: ImmortalSpace<VM>,
    #[fallback_trace]
    pub base: BasePlan<VM>,
}

impl<VM: VMBinding> CommonPlan<VM> {
    pub fn new(mut args: CreateSpecificPlanArgs<VM>) -> CommonPlan<VM> {
        CommonPlan {
            immortal: ImmortalSpace::new(args.get_space_args(
                "immortal",
                true,
                VMRequest::discontiguous(),
            )),
            los: LargeObjectSpace::new(
                args.get_space_args("los", true, VMRequest::discontiguous()),
                false,
            ),
            nonmoving: ImmortalSpace::new(args.get_space_args(
                "nonmoving",
                true,
                VMRequest::discontiguous(),
            )),
            base: BasePlan::new(args),
        }
    }

    pub fn get_spaces(&self) -> Vec<&dyn Space<VM>> {
        let mut ret = self.base.get_spaces();
        ret.push(&self.immortal);
        ret.push(&self.los);
        ret.push(&self.nonmoving);
        ret
    }

    pub fn get_used_pages(&self) -> usize {
        self.immortal.reserved_pages()
            + self.los.reserved_pages()
            + self.nonmoving.reserved_pages()
            + self.base.get_used_pages()
    }

    pub fn trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        if self.immortal.in_space(object) {
            trace!("trace_object: object in immortal space");
            return self.immortal.trace_object(queue, object);
        }
        if self.los.in_space(object) {
            trace!("trace_object: object in los");
            return self.los.trace_object(queue, object);
        }
        if self.nonmoving.in_space(object) {
            trace!("trace_object: object in nonmoving space");
            return self.nonmoving.trace_object(queue, object);
        }
        self.base.trace_object::<Q>(queue, object, worker)
    }

    pub fn prepare(&mut self, tls: VMWorkerThread, full_heap: bool) {
        self.immortal.prepare();
        self.los.prepare(full_heap);
        self.nonmoving.prepare();
        self.base.prepare(tls, full_heap)
    }

    pub fn release(&mut self, tls: VMWorkerThread, full_heap: bool) {
        self.immortal.release();
        self.los.release(full_heap);
        self.nonmoving.release();
        self.base.release(tls, full_heap)
    }

    pub fn stacks_prepared(&self) -> bool {
        self.base.stacks_prepared()
    }

    pub fn get_immortal(&self) -> &ImmortalSpace<VM> {
        &self.immortal
    }

    pub fn get_los(&self) -> &LargeObjectSpace<VM> {
        &self.los
    }

    pub fn get_nonmoving(&self) -> &ImmortalSpace<VM> {
        &self.nonmoving
    }

    pub(crate) fn verify_side_metadata_sanity(
        &self,
        side_metadata_sanity_checker: &mut SideMetadataSanity,
    ) {
        self.base
            .verify_side_metadata_sanity(side_metadata_sanity_checker);
        self.immortal
            .verify_side_metadata_sanity(side_metadata_sanity_checker);
        self.los
            .verify_side_metadata_sanity(side_metadata_sanity_checker);
        self.nonmoving
            .verify_side_metadata_sanity(side_metadata_sanity_checker);
    }
}

use crate::policy::gc_work::TraceKind;
use crate::vm::VMBinding;

/// A plan that uses `PlanProcessEdges` needs to provide an implementation for this trait.
/// Generally a plan does not need to manually implement this trait. Instead, we provide
/// a procedural macro that helps generate an implementation. Please check `macros/trace_object`.
///
/// A plan could also manually implement this trait. For the sake of performance, the implementation
/// of this trait should mark methods as `[inline(always)]`.
pub trait PlanTraceObject<VM: VMBinding> {
    /// Trace objects in the plan. Generally one needs to figure out
    /// which space an object resides in, and invokes the corresponding policy
    /// trace object method.
    ///
    /// Arguments:
    /// * `trace`: the current transitive closure
    /// * `object`: the object to trace. This is a non-nullable object reference.
    /// * `worker`: the GC worker that is tracing this object.
    fn trace_object<Q: ObjectQueue, const KIND: TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference;

    /// Post-scan objects in the plan. Each object is scanned by `VM::VMScanning::scan_object()`, and this function
    /// will be called after the `VM::VMScanning::scan_object()` as a hook to invoke possible policy post scan method.
    /// If a plan does not have any policy that needs post scan, this method can be implemented as empty.
    /// If a plan has a policy that has some policy specific behaviors for scanning (e.g. mark lines in Immix),
    /// this method should also invoke those policy specific methods for objects in that space.
    fn post_scan_object(&self, object: ObjectReference);

    /// Whether objects in this plan may move. If any of the spaces used by the plan may move objects, this should
    /// return true.
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}

use enum_map::Enum;
/// Allocation semantics that MMTk provides.
/// Each allocation request requires a desired semantic for the object to allocate.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Enum, PartialEq, Eq)]
pub enum AllocationSemantics {
    /// The default semantic. This means there is no specific requirement for the allocation.
    /// The actual semantic of the default will depend on the GC plan in use.
    Default = 0,
    /// Immortal objects will not be reclaimed. MMTk still traces immortal objects, but will not
    /// reclaim the objects even if they are dead.
    Immortal = 1,
    /// Large objects. It is usually desirable to allocate large objects specially. Large objects
    /// are allocated with page granularity and will not be moved.
    /// Each plan provides `max_non_los_default_alloc_bytes` (see [`crate::plan::PlanConstraints`]),
    /// which defines a threshold for objects that can be allocated with the default semantic. Any object that is larger than the
    /// threshold must be allocated with the `Los` semantic.
    /// This semantic may get removed and MMTk will transparently allocate into large object space for large objects.
    Los = 2,
    /// Code objects have execution permission.
    /// Note that this is a place holder for now. Currently all the memory MMTk allocates has execution permission.
    Code = 3,
    /// Read-only objects cannot be mutated once it is initialized.
    /// Note that this is a place holder for now. It does not provide read only semantic.
    ReadOnly = 4,
    /// Los + Code.
    LargeCode = 5,
    /// Non moving objects will not be moved by GC.
    NonMoving = 6,
}
