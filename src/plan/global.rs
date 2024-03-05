//! The global part of a plan implementation.

use super::PlanConstraints;
use crate::global_state::GlobalState;
use crate::mmtk::MMTK;
use crate::plan::tracing::ObjectQueue;
use crate::plan::Mutator;
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::{PlanCreateSpaceArgs, Space};
#[cfg(feature = "vm_space")]
use crate::policy::vmspace::VMSpace;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::{CopyConfig, GCWorkerCopyContext};
use crate::util::heap::gc_trigger::GCTrigger;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::layout::Mmapper;
use crate::util::heap::layout::VMMap;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataSanity;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::options::Options;
use crate::util::options::PlanSelector;
use crate::util::statistics::stats::Stats;
use crate::util::{conversions, ObjectReference};
use crate::util::{VMMutatorThread, VMWorkerThread};
use crate::vm::*;
use downcast_rs::Downcast;
use enum_map::EnumMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use mmtk_macros::{HasSpaces, PlanTraceObject};

pub fn create_mutator<VM: VMBinding>(
    tls: VMMutatorThread,
    mmtk: &'static MMTK<VM>,
) -> Box<Mutator<VM>> {
    Box::new(match *mmtk.options.plan {
        PlanSelector::NoGC => crate::plan::nogc::mutator::create_nogc_mutator(tls, mmtk),
        PlanSelector::SemiSpace => crate::plan::semispace::mutator::create_ss_mutator(tls, mmtk),
        PlanSelector::GenCopy => {
            crate::plan::generational::copying::mutator::create_gencopy_mutator(tls, mmtk)
        }
        PlanSelector::GenImmix => {
            crate::plan::generational::immix::mutator::create_genimmix_mutator(tls, mmtk)
        }
        PlanSelector::MarkSweep => crate::plan::marksweep::mutator::create_ms_mutator(tls, mmtk),
        PlanSelector::Immix => crate::plan::immix::mutator::create_immix_mutator(tls, mmtk),
        PlanSelector::PageProtect => {
            crate::plan::pageprotect::mutator::create_pp_mutator(tls, mmtk)
        }
        PlanSelector::MarkCompact => {
            crate::plan::markcompact::mutator::create_markcompact_mutator(tls, mmtk)
        }
        PlanSelector::StickyImmix => {
            crate::plan::sticky::immix::mutator::create_stickyimmix_mutator(tls, mmtk)
        }
    })
}

pub fn create_plan<VM: VMBinding>(
    plan: PlanSelector,
    args: CreateGeneralPlanArgs<VM>,
) -> Box<dyn Plan<VM = VM>> {
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
        PlanSelector::StickyImmix => {
            Box::new(crate::plan::sticky::immix::StickyImmix::new(args)) as Box<dyn Plan<VM = VM>>
        }
    };

    // We have created Plan in the heap, and we won't explicitly move it.
    // Each space now has a fixed address for its lifetime. It is safe now to initialize SFT.
    let sft_map: &mut dyn crate::policy::sft_map::SFTMap =
        unsafe { crate::mmtk::SFT_MAP.get_mut() }.as_mut();
    plan.for_each_space(&mut |s| {
        sft_map.notify_space_creation(s.as_sft());
        s.initialize_sft(sft_map);
    });

    plan
}

/// Create thread local GC worker.
pub fn create_gc_worker_context<VM: VMBinding>(
    tls: VMWorkerThread,
    mmtk: &'static MMTK<VM>,
) -> GCWorkerCopyContext<VM> {
    GCWorkerCopyContext::<VM>::new(tls, mmtk, mmtk.get_plan().create_copy_config())
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
/// 4. Invoke the `verify_side_metadata_sanity()` method of the plan.
///    It will create a `SideMetadataSanity` object, and invoke verify_side_metadata_sanity() for each space (or
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
pub trait Plan: 'static + HasSpaces + Sync + Downcast {
    /// Get the plan constraints for the plan.
    /// This returns a non-constant value. A constant value can be found in each plan's module if needed.
    fn constraints(&self) -> &'static PlanConstraints;

    /// Create a copy config for this plan. A copying GC plan MUST override this method,
    /// and provide a valid config.
    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        // Use the empty default copy config for non copying GC.
        CopyConfig::default()
    }

    /// Get a immutable reference to the base plan. `BasePlan` is included by all the MMTk GC plans.
    fn base(&self) -> &BasePlan<Self::VM>;

    /// Get a mutable reference to the base plan. `BasePlan` is included by all the MMTk GC plans.
    fn base_mut(&mut self) -> &mut BasePlan<Self::VM>;

    /// Schedule work for the upcoming GC.
    fn schedule_collection(&'static self, _scheduler: &GCWorkScheduler<Self::VM>);

    /// Get the common plan. CommonPlan is included by most of MMTk GC plans.
    fn common(&self) -> &CommonPlan<Self::VM> {
        panic!("Common Plan not handled!")
    }

    /// Return a reference to `GenerationalPlan` to allow
    /// access methods specific to generational plans if the plan is a generational plan.
    fn generational(
        &self,
    ) -> Option<&dyn crate::plan::generational::global::GenerationalPlan<VM = Self::VM>> {
        None
    }

    /// Get the current run time options.
    fn options(&self) -> &Options {
        &self.base().options
    }

    /// Get the allocator mapping between [`crate::AllocationSemantics`] and [`crate::util::alloc::AllocatorSelector`].
    /// This defines what space this plan will allocate objects into for different semantics.
    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector>;

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

    /// Notify the plan that an emergency collection will happen. The plan should try to free as much memory as possible.
    /// The default implementation will force a full heap collection for generational plans.
    fn notify_emergency_collection(&self) {
        if let Some(gen) = self.generational() {
            gen.force_full_heap_collection();
        }
    }

    /// Ask the plan if they would trigger a GC. If MMTk is in charge of triggering GCs, this method is called
    /// periodically during allocation. However, MMTk may delegate the GC triggering decision to the runtime,
    /// in which case, this method may not be called. This method returns true to trigger a collection.
    ///
    /// # Arguments
    /// * `space_full`: the allocation to a specific space failed, must recover pages within 'space'.
    /// * `space`: an option to indicate if there is a space that has failed in an allocation.
    fn collection_required(&self, space_full: bool, space: Option<SpaceStats<Self::VM>>) -> bool;

    // Note: The following methods are about page accounting. The default implementation should
    // work fine for non-copying plans. For copying plans, the plan should override any of these methods
    // if necessary.

    /// Get the number of pages that are reserved, including pages used by MMTk spaces, pages that
    /// will be used (e.g. for copying), and live pages allocated outside MMTk spaces as reported
    /// by the VM binding.
    fn get_reserved_pages(&self) -> usize {
        let used_pages = self.get_used_pages();
        let collection_reserve = self.get_collection_reserved_pages();
        let vm_live_bytes = <Self::VM as VMBinding>::VMCollection::vm_live_bytes();
        // Note that `vm_live_bytes` may not be the exact number of bytes in whole pages.  The VM
        // binding is allowed to return an approximate value if it is expensive or impossible to
        // compute the exact number of pages occupied.
        let vm_live_pages = conversions::bytes_to_pages_up(vm_live_bytes);
        let total = used_pages + collection_reserve + vm_live_pages;

        trace!(
            "Reserved pages = {}, used pages: {}, collection reserve: {}, VM live pages: {}",
            total,
            used_pages,
            collection_reserve,
            vm_live_pages,
        );

        total
    }

    /// Get the total number of pages for the heap.
    fn get_total_pages(&self) -> usize {
        self.base()
            .gc_trigger
            .policy
            .get_current_heap_size_in_pages()
    }

    /// Get the number of pages that are still available for use. The available pages
    /// should always be positive or 0.
    fn get_available_pages(&self) -> usize {
        let reserved_pages = self.get_reserved_pages();
        let total_pages = self.get_total_pages();

        // It is possible that the reserved pages is larger than the total pages so we are doing
        // a saturating subtraction to make sure we return a non-negative number.
        // For example,
        // 1. our GC trigger checks if reserved pages is more than total pages.
        // 2. when the heap is almost full of live objects (such as in the case of an OOM) and we are doing a copying GC, it is possible
        //    the reserved pages is larger than total pages after the copying GC (the reserved pages after a GC
        //    may be larger than the reserved pages before a GC, as we may end up using more memory for thread local
        //    buffers for copy allocators).
        let available_pages = total_pages.saturating_sub(reserved_pages);
        trace!(
            "Total pages = {}, reserved pages = {}, available pages = {}",
            total_pages,
            reserved_pages,
            available_pages,
        );
        available_pages
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

    /// Return whether last GC was an exhaustive attempt to collect the heap.
    /// For example, for generational GCs, minor collection is not an exhaustive collection.
    /// For example, for Immix, fast collection (no defragmentation) is not an exhaustive collection.
    fn last_collection_was_exhaustive(&self) -> bool {
        true
    }

    /// An object is firstly reached by a sanity GC. So the object is reachable
    /// in the current GC, and all the GC work has been done for the object (such as
    /// tracing and releasing). A plan can implement this to
    /// use plan specific semantics to check if the object is sane.
    /// Return true if the object is considered valid by the plan.
    fn sanity_check_object(&self, _object: ObjectReference) -> bool {
        true
    }

    /// Call `space.verify_side_metadata_sanity` for all spaces in this plan.
    fn verify_side_metadata_sanity(&self) {
        let mut side_metadata_sanity_checker = SideMetadataSanity::new();
        self.for_each_space(&mut |space| {
            space.verify_side_metadata_sanity(&mut side_metadata_sanity_checker);
        })
    }

    /// Dump memory stats for the plan
    #[cfg(feature = "dump_memory_stats")]
    fn dump_memory_stats(&self) {}
}

impl_downcast!(Plan assoc VM);

/**
BasePlan should contain all plan-related state and functions that are _fundamental_ to _all_ plans.  These include VM-specific (but not plan-specific) features such as a code space or vm space, which are fundamental to all plans for a given VM.  Features that are common to _many_ (but not intrinsically _all_) plans should instead be included in CommonPlan.
*/
#[derive(HasSpaces, PlanTraceObject)]
pub struct BasePlan<VM: VMBinding> {
    pub(crate) global_state: Arc<GlobalState>,
    pub options: Arc<Options>,
    pub gc_trigger: Arc<GCTrigger<VM>>,

    // Spaces in base plan
    #[cfg(feature = "code_space")]
    #[space]
    pub code_space: ImmortalSpace<VM>,
    #[cfg(feature = "code_space")]
    #[space]
    pub code_lo_space: ImmortalSpace<VM>,
    #[cfg(feature = "ro_space")]
    #[space]
    pub ro_space: ImmortalSpace<VM>,

    /// A VM space is a space allocated and populated by the VM.  Currently it is used by JikesRVM
    /// for boot image.
    ///
    /// If VM space is present, it has some special interaction with the
    /// `memory_manager::is_mmtk_object` and the `memory_manager::is_in_mmtk_spaces` functions.
    ///
    /// -   The `is_mmtk_object` funciton requires the valid object (VO) bit side metadata to identify objects,
    ///     but currently we do not require the boot image to provide it, so it will not work if the
    ///     address argument is in the VM space.
    ///
    /// -   The `is_in_mmtk_spaces` currently returns `true` if the given object reference is in
    ///     the VM space.
    #[cfg(feature = "vm_space")]
    #[space]
    pub vm_space: VMSpace<VM>,
}

/// Args needed for creating any plan. This includes a set of contexts from MMTK or global. This
/// is passed to each plan's constructor.
pub struct CreateGeneralPlanArgs<'a, VM: VMBinding> {
    pub vm_map: &'static dyn VMMap,
    pub mmapper: &'static dyn Mmapper,
    pub options: Arc<Options>,
    pub state: Arc<GlobalState>,
    pub gc_trigger: Arc<crate::util::heap::gc_trigger::GCTrigger<VM>>,
    pub scheduler: Arc<GCWorkScheduler<VM>>,
    pub stats: &'a Stats,
    pub heap: &'a mut HeapMeta,
}

/// Args needed for creating a specific plan. This includes plan-specific args, such as plan constrainst
/// and their global side metadata specs. This is created in each plan's constructor, and will be passed
/// to `CommonPlan` or `BasePlan`. Also you can create `PlanCreateSpaceArg` from this type, and use that
/// to create spaces.
pub struct CreateSpecificPlanArgs<'a, VM: VMBinding> {
    pub global_args: CreateGeneralPlanArgs<'a, VM>,
    pub constraints: &'static PlanConstraints,
    pub global_side_metadata_specs: Vec<SideMetadataSpec>,
}

impl<'a, VM: VMBinding> CreateSpecificPlanArgs<'a, VM> {
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
            heap: self.global_args.heap,
            constraints: self.constraints,
            gc_trigger: self.global_args.gc_trigger.clone(),
            scheduler: self.global_args.scheduler.clone(),
            options: &self.global_args.options,
            global_state: self.global_args.state.clone(),
        }
    }
}

impl<VM: VMBinding> BasePlan<VM> {
    #[allow(unused_mut)] // 'args' only needs to be mutable for certain features
    pub fn new(mut args: CreateSpecificPlanArgs<VM>) -> BasePlan<VM> {
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
            vm_space: VMSpace::new(args.get_space_args(
                "vm_space",
                false,
                VMRequest::discontiguous(),
            )),

            global_state: args.global_args.state.clone(),
            gc_trigger: args.global_args.gc_trigger,
            options: args.global_args.options,
        }
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
            pages += self.global_state.get_malloc_bytes_in_pages();
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

    pub(crate) fn collection_required<P: Plan>(&self, plan: &P, space_full: bool) -> bool {
        let stress_force_gc =
            crate::util::heap::gc_trigger::GCTrigger::<VM>::should_do_stress_gc_inner(
                &self.global_state,
                &self.options,
            );
        if stress_force_gc {
            debug!(
                "Stress GC: allocation_bytes = {}, stress_factor = {}",
                self.global_state.allocation_bytes.load(Ordering::Relaxed),
                *self.options.stress_factor
            );
            debug!("Doing stress GC");
            self.global_state
                .allocation_bytes
                .store(0, Ordering::SeqCst);
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
}

/**
CommonPlan is for representing state and features used by _many_ plans, but that are not fundamental to _all_ plans.  Examples include the Large Object Space and an Immortal space.  Features that are fundamental to _all_ plans must be included in BasePlan.
*/
#[derive(HasSpaces, PlanTraceObject)]
pub struct CommonPlan<VM: VMBinding> {
    #[space]
    pub immortal: ImmortalSpace<VM>,
    #[space]
    pub los: LargeObjectSpace<VM>,
    // TODO: We should use a marksweep space for nonmoving.
    #[space]
    pub nonmoving: ImmortalSpace<VM>,
    #[parent]
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

    pub fn get_immortal(&self) -> &ImmortalSpace<VM> {
        &self.immortal
    }

    pub fn get_los(&self) -> &LargeObjectSpace<VM> {
        &self.los
    }

    pub fn get_nonmoving(&self) -> &ImmortalSpace<VM> {
        &self.nonmoving
    }
}

use crate::policy::gc_work::TraceKind;
use crate::vm::VMBinding;

/// A trait for anything that contains spaces.
/// Examples include concrete plans as well as `Gen`, `CommonPlan` and `BasePlan`.
/// All plans must implement this trait.
///
/// This trait provides methods for enumerating spaces in a struct, including spaces in nested
/// struct.
///
/// This trait can be implemented automatically by adding the `#[derive(HasSpaces)]` attribute to a
/// struct.  It uses the derive macro defined in the `mmtk-macros` crate.
///
/// This trait visits spaces as `dyn`, so it should only be used when performance is not critical.
/// For performance critical methods that visit spaces in a plan, such as `trace_object`, it is
/// recommended to define a trait (such as `PlanTraceObject`) for concrete plans to implement, and
/// implement (by hand or automatically) the method without `dyn`.
pub trait HasSpaces {
    // The type of the VM.
    type VM: VMBinding;

    /// Visit each space field immutably.
    ///
    /// If `Self` contains nested fields that contain more spaces, this method shall visit spaces
    /// in the outer struct first.
    fn for_each_space(&self, func: &mut dyn FnMut(&dyn Space<Self::VM>));

    /// Visit each space field mutably.
    ///
    /// If `Self` contains nested fields that contain more spaces, this method shall visit spaces
    /// in the outer struct first.
    fn for_each_space_mut(&mut self, func: &mut dyn FnMut(&mut dyn Space<Self::VM>));
}

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
