use crate::plan::concurrent::concurrent_marking_work::ProcessRootSlots;
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::immix::gc_work::ConcurrentImmixGCWorkContext;
use crate::plan::concurrent::immix::gc_work::ConcurrentImmixSTWGCWorkContext;
use crate::plan::concurrent::Pause;
use crate::plan::global::BasePlan;
use crate::plan::global::CommonPlan;
use crate::plan::global::CreateGeneralPlanArgs;
use crate::plan::global::CreateSpecificPlanArgs;
use crate::plan::immix::mutator::ALLOCATOR_MAPPING;
use crate::plan::AllocationSemantics;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::ImmixSpaceArgs;
use crate::policy::immix::TRACE_KIND_DEFRAG;
use crate::policy::immix::TRACE_KIND_FAST;
use crate::policy::space::Space;
use crate::scheduler::gc_work::Release;
use crate::scheduler::gc_work::StopMutators;
use crate::scheduler::gc_work::UnsupportedProcessEdges;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
use crate::util::copy::*;
use crate::util::heap::gc_trigger::SpaceStats;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::vm::ObjectModel;
use crate::vm::VMBinding;
use crate::{policy::immix::ImmixSpace, util::opaque_pointer::VMWorkerThread};
use std::sync::atomic::AtomicBool;

use atomic::Atomic;
use atomic::Ordering;
use enum_map::EnumMap;

use mmtk_macros::{HasSpaces, PlanTraceObject};

#[derive(Debug, Clone, Copy, bytemuck::NoUninit, PartialEq, Eq)]
#[repr(u8)]
enum GCCause {
    Unknown,
    FullHeap,
    InitialMark,
    FinalMark,
}

/// A concurrent Immix plan. The plan supports concurrent collection (strictly non-moving) and STW full heap collection (which may do defrag).
/// The concurrent GC consists of two STW pauses (initial mark and final mark) with concurrent marking in between.
#[derive(HasSpaces, PlanTraceObject)]
pub struct ConcurrentImmix<VM: VMBinding> {
    #[post_scan]
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub immix_space: ImmixSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
    last_gc_was_defrag: AtomicBool,
    current_pause: Atomic<Option<Pause>>,
    previous_pause: Atomic<Option<Pause>>,
    gc_cause: Atomic<GCCause>,
    concurrent_marking_active: AtomicBool,
}

/// The plan constraints for the immix plan.
pub const CONCURRENT_IMMIX_CONSTRAINTS: PlanConstraints = PlanConstraints {
    // If we disable moving in Immix, this is a non-moving plan.
    moves_objects: !cfg!(feature = "immix_non_moving"),
    // Max immix object size is half of a block.
    max_non_los_default_alloc_bytes: crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
    needs_prepare_mutator: true,
    barrier: crate::BarrierSelector::SATBBarrier,
    needs_log_bit: true,
    ..PlanConstraints::default()
};

impl<VM: VMBinding> Plan for ConcurrentImmix<VM> {
    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        if self.base().collection_required(self, space_full) {
            self.gc_cause.store(GCCause::FullHeap, Ordering::Release);
            return true;
        }

        let concurrent_marking_in_progress = self.concurrent_marking_in_progress();

        if concurrent_marking_in_progress
            && self.common.base.scheduler.work_buckets[WorkBucketStage::Concurrent].is_drained()
        {
            self.gc_cause.store(GCCause::FinalMark, Ordering::Release);
            return true;
        }
        let threshold = self.get_total_pages() >> 1;
        let used_pages_after_last_gc = self.common.base.global_state.get_used_pages_after_last_gc();
        let used_pages_now = self.get_used_pages();
        let allocated = used_pages_now.saturating_sub(used_pages_after_last_gc);
        if !concurrent_marking_in_progress && allocated > threshold {
            info!("Allocated {allocated} pages since last GC ({used_pages_now} - {used_pages_after_last_gc} > {threshold}): Do concurrent marking");
            debug_assert!(
                self.common.base.scheduler.work_buckets[WorkBucketStage::Concurrent].is_empty()
            );
            debug_assert!(!self.concurrent_marking_in_progress());
            let prev_pause = self.previous_pause();
            debug_assert!(prev_pause.is_none() || prev_pause.unwrap() != Pause::InitialMark);
            self.gc_cause.store(GCCause::InitialMark, Ordering::Release);
            return true;
        }
        false
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        self.immix_space
            .is_last_gc_exhaustive(self.last_gc_was_defrag.load(Ordering::Relaxed))
    }

    fn constraints(&self) -> &'static PlanConstraints {
        &CONCURRENT_IMMIX_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<Self::VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::Immix(0), &self.immix_space)],
            constraints: &CONCURRENT_IMMIX_CONSTRAINTS,
        }
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        let pause = self.select_collection_kind();
        if pause == Pause::Full {
            self.current_pause
                .store(Some(Pause::Full), Ordering::SeqCst);
            self.set_ref_closure_buckets_enabled(true);
            crate::plan::immix::global::Immix::schedule_immix_full_heap_collection::<
                ConcurrentImmix<VM>,
                ConcurrentImmixSTWGCWorkContext<VM, TRACE_KIND_FAST>,
                ConcurrentImmixSTWGCWorkContext<VM, TRACE_KIND_DEFRAG>,
            >(self, &self.immix_space, scheduler);
        } else {
            // Set current pause kind
            self.current_pause.store(Some(pause), Ordering::SeqCst);
            // Schedule work
            match pause {
                Pause::InitialMark => self.schedule_concurrent_marking_initial_pause(scheduler),
                Pause::FinalMark => self.schedule_concurrent_marking_final_pause(scheduler),
                _ => unreachable!(),
            }
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        match pause {
            Pause::Full => {
                self.common.prepare(tls, true);
                self.immix_space.prepare(
                    true,
                    Some(crate::policy::immix::defrag::StatsForDefrag::new(self)),
                );
            }
            Pause::InitialMark => {
                // init prepare has to be executed first, otherwise, los objects will not be
                // dealt with properly
                // self.common.initial_pause_prepare();
                // self.immix_space.initial_pause_prepare();
                if VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.is_on_side() {
                    self.common.set_side_log_bits();
                    self.immix_space.set_side_log_bits();
                }
                self.common.prepare(tls, true);
                self.immix_space.prepare(
                    true,
                    Some(crate::policy::immix::defrag::StatsForDefrag::new(self)),
                );
            }
            Pause::FinalMark => (),
        }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        match pause {
            Pause::InitialMark => (),
            Pause::Full | Pause::FinalMark => {
                if VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.is_on_side() {
                    self.immix_space.clear_side_log_bits();
                    self.common.clear_side_log_bits();
                }
                // self.immix_space.final_pause_release();
                // self.common.final_pause_release();
                self.common.release(tls, true);
                // release the collected region
                self.immix_space.release(true);
            }
        }
    }

    fn end_of_gc(&mut self, _tls: VMWorkerThread) {
        self.last_gc_was_defrag
            .store(self.immix_space.end_of_gc(), Ordering::Relaxed);

        let pause = self.current_pause().unwrap();
        if pause == Pause::InitialMark {
            self.set_concurrent_marking_state(true);
        }
        self.previous_pause.store(Some(pause), Ordering::SeqCst);
        self.current_pause.store(None, Ordering::SeqCst);
        info!("{:?} end", pause);
    }

    fn current_gc_may_move_object(&self) -> bool {
        self.immix_space.in_defrag()
    }

    fn get_collection_reserved_pages(&self) -> usize {
        self.immix_space.defrag_headroom_pages()
    }

    fn get_used_pages(&self) -> usize {
        self.immix_space.reserved_pages() + self.common.get_used_pages()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<Self::VM> {
        &mut self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn notify_mutators_paused(&self, _scheduler: &GCWorkScheduler<VM>) {
        use crate::vm::ActivePlan;
        let pause = self.current_pause().unwrap();
        match pause {
            Pause::Full => {
                self.set_concurrent_marking_state(false);
            }
            Pause::InitialMark => {
                debug_assert!(
                    !self.concurrent_marking_in_progress(),
                    "prev pause: {:?}",
                    self.previous_pause().unwrap()
                );
            }
            Pause::FinalMark => {
                debug_assert!(self.concurrent_marking_in_progress());
                // Flush barrier buffers
                for mutator in <VM as VMBinding>::VMActivePlan::mutators() {
                    mutator.barrier.flush();
                }
                self.set_concurrent_marking_state(false);
            }
        }
        // scheduler.work_buckets[WorkBucketStage::Concurrent].close();
        info!("{:?} start", pause);
    }

    fn concurrent(&self) -> Option<&dyn ConcurrentPlan<VM = VM>> {
        Some(self)
    }
}

impl<VM: VMBinding> ConcurrentImmix<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Self {
        let spec = crate::util::metadata::extract_side_metadata(&[
            *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
        ]);

        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &CONCURRENT_IMMIX_CONSTRAINTS,
            global_side_metadata_specs: SideMetadataContext::new_global_specs(&spec),
        };

        let immix_args = ImmixSpaceArgs {
            mixed_age: false,
            never_move_objects: false,
        };

        // These buckets are not used in an Immix plan. We can simply disable them.
        // TODO: We should be more systmatic on this, and disable unnecessary buckets for other plans as well.
        let scheduler = &plan_args.global_args.scheduler;
        scheduler.work_buckets[WorkBucketStage::VMRefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::CalculateForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::SecondRoots].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::RefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::FinalizableForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::Compact].set_enabled(false);

        let immix = ConcurrentImmix {
            immix_space: ImmixSpace::new(
                plan_args.get_normal_space_args("immix", true, false, VMRequest::discontiguous()),
                immix_args,
            ),
            common: CommonPlan::new(plan_args),
            last_gc_was_defrag: AtomicBool::new(false),
            current_pause: Atomic::new(None),
            previous_pause: Atomic::new(None),
            gc_cause: Atomic::new(GCCause::Unknown),
            concurrent_marking_active: AtomicBool::new(false),
        };

        immix.verify_side_metadata_sanity();

        immix
    }

    fn select_collection_kind(&self) -> Pause {
        match self.gc_cause.load(Ordering::Acquire) {
            GCCause::FullHeap => Pause::Full,
            GCCause::InitialMark => Pause::InitialMark,
            GCCause::FinalMark => Pause::FinalMark,
            GCCause::Unknown => {
                panic!("Collection kind is not set when scheduling a collection");
            }
        }
    }

    fn set_ref_closure_buckets_enabled(&self, do_closure: bool) {
        let scheduler = &self.common.base.scheduler;
        scheduler.work_buckets[WorkBucketStage::VMRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::WeakRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::FinalRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::SoftRefClosure].set_enabled(do_closure);
        scheduler.work_buckets[WorkBucketStage::PhantomRefClosure].set_enabled(do_closure);
    }

    pub(crate) fn schedule_concurrent_marking_initial_pause(
        &'static self,
        scheduler: &GCWorkScheduler<VM>,
    ) {
        use crate::scheduler::gc_work::Prepare;

        self.set_ref_closure_buckets_enabled(false);

        scheduler.work_buckets[WorkBucketStage::Unconstrained].add(StopMutators::<
            ConcurrentImmixGCWorkContext<ProcessRootSlots<VM, Self, TRACE_KIND_FAST>>,
        >::new());
        scheduler.work_buckets[WorkBucketStage::Prepare].add(Prepare::<
            ConcurrentImmixGCWorkContext<UnsupportedProcessEdges<VM>>,
        >::new(self));
    }

    fn schedule_concurrent_marking_final_pause(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.set_ref_closure_buckets_enabled(true);

        // Skip root scanning in the final mark
        scheduler.work_buckets[WorkBucketStage::Unconstrained].add(StopMutators::<
            ConcurrentImmixGCWorkContext<ProcessRootSlots<VM, Self, TRACE_KIND_FAST>>,
        >::new_no_scan_roots());

        scheduler.work_buckets[WorkBucketStage::Release].add(Release::<
            ConcurrentImmixGCWorkContext<UnsupportedProcessEdges<VM>>,
        >::new(self));

        // Deal with weak ref and finalizers
        // TODO: Check against schedule_common_work and see if we are still missing any work packet
        type RefProcessingEdges<VM> =
            crate::scheduler::gc_work::PlanProcessEdges<VM, ConcurrentImmix<VM>, TRACE_KIND_FAST>;
        // Reference processing
        if !*self.base().options.no_reference_types {
            use crate::util::reference_processor::{
                PhantomRefProcessing, SoftRefProcessing, WeakRefProcessing,
            };
            scheduler.work_buckets[WorkBucketStage::SoftRefClosure]
                .add(SoftRefProcessing::<RefProcessingEdges<VM>>::new());
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure]
                .add(WeakRefProcessing::<VM>::new());
            scheduler.work_buckets[WorkBucketStage::PhantomRefClosure]
                .add(PhantomRefProcessing::<VM>::new());

            use crate::util::reference_processor::RefEnqueue;
            scheduler.work_buckets[WorkBucketStage::Release].add(RefEnqueue::<VM>::new());
        }

        // Finalization
        if !*self.base().options.no_finalizer {
            use crate::util::finalizable_processor::Finalization;
            // finalization
            scheduler.work_buckets[WorkBucketStage::FinalRefClosure]
                .add(Finalization::<RefProcessingEdges<VM>>::new());
        }
    }

    pub fn concurrent_marking_in_progress(&self) -> bool {
        self.concurrent_marking_active.load(Ordering::Acquire)
    }

    fn set_concurrent_marking_state(&self, active: bool) {
        use crate::plan::global::HasSpaces;
        use crate::vm::Collection;

        // Update the binding about concurrent marking
        <VM as VMBinding>::VMCollection::set_concurrent_marking_state(active);

        // Tell the spaces to allocate new objects as live
        let allocate_object_as_live = active;
        self.for_each_space(&mut |space: &dyn Space<VM>| {
            space.set_allocate_as_live(allocate_object_as_live);
        });

        // Store the state.
        self.concurrent_marking_active
            .store(active, Ordering::SeqCst);
    }

    fn previous_pause(&self) -> Option<Pause> {
        self.previous_pause.load(Ordering::SeqCst)
    }
}

impl<VM: VMBinding> ConcurrentPlan for ConcurrentImmix<VM> {
    fn current_pause(&self) -> Option<Pause> {
        self.current_pause.load(Ordering::SeqCst)
    }

    fn concurrent_work_in_progress(&self) -> bool {
        self.concurrent_marking_in_progress()
    }
}
