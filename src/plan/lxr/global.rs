use super::gc_work::{LXRGCWorkContext, LXRWeakRefWorkContext, ReleaseLOSNursery};
use super::mutator::ALLOCATOR_MAPPING;
use super::rc::{ProcessDecs, RCImmixCollectRootEdges};
use super::remset::FlushMatureEvacRemsets;
use super::{LazySweepingJobsCounter, LAZY_SWEEPING_JOBS};
use crate::plan::global::CommonPlan;
use crate::plan::global::{BasePlan, CreateGeneralPlanArgs, CreateSpecificPlanArgs};
use crate::plan::immix::Pause;
use crate::plan::lxr::gc_work::FastRCPrepare;
use crate::plan::AllocationSemantics;
use crate::plan::MutatorContext;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::block::Block;
use crate::policy::immix::ImmixSpaceArgs;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(feature = "analysis")]
use crate::util::analysis::GcHookWork;
use crate::util::constants::*;
use crate::util::copy::*;
use crate::util::heap::{SpaceStats, VMRequest};
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::MetadataSpec;
use crate::util::options::Options;
use crate::util::rc::{RefCountHelper, RC_TABLE};
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::{metadata, Address, ObjectReference};
use crate::vm::ActivePlan;
use crate::vm::{Collection, ObjectModel, VMBinding};
use crate::BarrierSelector;
use crate::{policy::immix::ImmixSpace, util::opaque_pointer::VMWorkerThread};
use atomic::{Atomic, Ordering};
use crossbeam::queue::SegQueue;
use enum_map::EnumMap;
use spin::Lazy;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Condvar, Mutex, RwLock};

const LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER: usize = 1;

static HEAP_AFTER_GC: AtomicUsize = AtomicUsize::new(0);

use mmtk_macros::{HasSpaces, PlanTraceObject};

#[derive(HasSpaces, PlanTraceObject)]
pub struct LXR<VM: VMBinding> {
    #[post_scan]
    #[space]
    #[copy_semantics(CopySemantics::DefaultCopy)]
    pub immix_space: ImmixSpace<VM>,
    #[parent]
    pub common: CommonPlan<VM>,
    /// Always true for non-rc immix.
    /// For RC immix, this is used for enable backup tracing.
    perform_cycle_collection: AtomicBool,
    current_pause: Atomic<Option<Pause>>,
    previous_pause: Atomic<Option<Pause>>,
    hint_cycle_gc: AtomicBool,
    hint_emergency_gc: AtomicBool,
    last_gc_was_defrag: AtomicBool,
    avail_pages_at_end_of_last_gc: AtomicUsize,
    zeroing_packets_scheduled: AtomicBool,
    decide_cycle_collection: (Mutex<bool>, Condvar),
    in_concurrent_marking: AtomicBool,
    pub prev_roots: RwLock<SegQueue<Vec<ObjectReference>>>,
    pub curr_roots: RwLock<SegQueue<Vec<ObjectReference>>>,
    pub rc: RefCountHelper<VM>,
}

pub static LXR_CONSTRAINTS: Lazy<PlanConstraints> = Lazy::new(|| PlanConstraints {
    moves_objects: true,
    // Max immix object size is half of a block.
    max_non_los_default_alloc_bytes: crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
    barrier: BarrierSelector::FieldBarrier,
    needs_log_bit: true,
    needs_field_log_bit: true,
    rc_enabled: true,
    ..PlanConstraints::default()
});

impl<VM: VMBinding> Plan for LXR<VM> {
    fn current_gc_may_move_object(&self) -> bool {
        true
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        // Spaces or heap full
        if self.base().collection_required(self, space_full) {
            return true;
        }
        // SATB is finished
        if self.cm_in_progress() && super::concurrent_marking_packets_drained() {
            return true;
        }
        // Survival limits
        let total_young_alloc_pages = self
            .immix_space
            .block_allocation
            .total_young_allocation_in_bytes()
            >> LOG_BYTES_IN_MBYTE;
        let predicted_survival_mb: usize =
            ((total_young_alloc_pages as f64 * super::SURVIVAL_RATIO_PREDICTOR.ratio()) as usize)
                << LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER;
        if predicted_survival_mb >= super::MAX_SURVIVAL_MB {
            return true;
        }
        if !self.immix_space.common().contiguous {
            let available_to_space = self.get_total_pages() - self.get_used_pages();
            if predicted_survival_mb >= available_to_space {
                return true;
            }
        }
        false
    }

    fn concurrent_collection_required(&self) -> bool {
        return false;
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        let x = self.previous_pause.load(Ordering::SeqCst);
        x == Some(Pause::Full) || x == Some(Pause::FullDefrag)
    }

    fn constraints(&self) -> &'static PlanConstraints {
        &LXR_CONSTRAINTS
    }

    fn create_copy_config(&'static self) -> CopyConfig<VM> {
        use enum_map::enum_map;
        CopyConfig {
            copy_mapping: enum_map! {
                CopySemantics::DefaultCopy => CopySelector::Immix(0),
                _ => CopySelector::Unused,
            },
            space_mapping: vec![(CopySelector::Immix(0), &self.immix_space)],
            constraints: &LXR_CONSTRAINTS,
        }
    }

    fn schedule_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        if !super::LazySweepingJobs::all_finished() {
            warn!("LXR Lazy Sweeping Not Finished");
        }
        let pause = self.select_collection_kind();
        // Wait for concurrent packets
        // Mark table zeroing
        if pause == Pause::InitialMark || pause == Pause::Full {
            self.schedule_mark_table_zeroing_tasks(Some(pause))
        }
        self.zeroing_packets_scheduled
            .store(false, Ordering::SeqCst);
        // Set current pause kind
        self.current_pause.store(Some(pause), Ordering::SeqCst);
        self.perform_cycle_collection
            .store(pause != Pause::RefCount, Ordering::SeqCst);
        // Schedule work
        match pause {
            Pause::Full => self
                .schedule_emergency_full_heap_collection::<RCImmixCollectRootEdges<VM>>(scheduler),
            Pause::FullDefrag => unreachable!(),
            Pause::RefCount => self.schedule_rc_collection(scheduler),
            Pause::InitialMark => self.schedule_concurrent_marking_initial_pause(scheduler),
            Pause::FinalMark => self.schedule_concurrent_marking_final_pause(scheduler),
        }
        // Analysis routine that is ran. It is generally recommended to take advantage
        // of the scheduling system we have in place for more performance
        #[cfg(feature = "analysis")]
        scheduler.work_buckets[WorkBucketStage::Unconstrained].add(GcHookWork);
        // Resume mutators
        if pause == Pause::Full || pause == Pause::FinalMark {
            #[cfg(feature = "sanity")]
            scheduler.work_buckets[WorkBucketStage::Final].add(ScheduleSanityGC::<Self>::new(self));
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        if pause == Pause::FinalMark || pause == Pause::Full {
            self.common.los.is_end_of_satb_or_full_gc = true;
            // release nursery memory before mature evacuation, to reduce the chance of to-space overflow.
            self.immix_space.scheduler().work_buckets[WorkBucketStage::Unconstrained]
                .add(ReleaseLOSNursery);
        }
        self.common
            .prepare(tls, pause == Pause::Full || pause == Pause::InitialMark);
        if super::MATURE_EVACUATION && (pause == Pause::FinalMark || pause == Pause::Full) {
            self.immix_space.process_mature_evacuation_remset();
            self.immix_space.scheduler().work_buckets[WorkBucketStage::RCEvacuateMature]
                .add(FlushMatureEvacRemsets);
        }
        self.immix_space.prepare_rc(pause);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        let _new_ratio = super::SURVIVAL_RATIO_PREDICTOR.update_ratio();
        let pause = self.current_pause().unwrap();
        if pause == Pause::FinalMark || pause == Pause::Full {
            VM::VMCollection::update_weak_processor(false);
        }
        <VM as VMBinding>::VMCollection::vm_release();
        self.common.los.is_end_of_satb_or_full_gc = false;
        self.common
            .release(tls, pause == Pause::Full || pause == Pause::FinalMark);
        self.immix_space.release_rc(pause);
        // swap roots
        let mut prev_roots = self.prev_roots.write().unwrap();
        let mut curr_roots = self.curr_roots.write().unwrap();
        std::mem::swap::<SegQueue<_>>(&mut prev_roots, &mut curr_roots);
        debug_assert!(curr_roots.is_empty());
        // release the collected region
        self.last_gc_was_defrag.store(
            self.current_pause().unwrap() == Pause::FullDefrag,
            Ordering::Relaxed,
        );
    }

    fn get_collection_reserved_pages(&self) -> usize {
        let survival = {
            let predicted_survival = (self.immix_space.block_allocation.clean_nursery_mb() as f64
                * super::SURVIVAL_RATIO_PREDICTOR.ratio())
                as usize;
            predicted_survival << LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER
        };
        return survival + self.immix_space.defrag_headroom_pages();
    }

    fn get_used_pages(&self) -> usize {
        self.immix_space.reserved_pages() + self.common.get_used_pages()
    }

    fn base(&self) -> &BasePlan<VM> {
        &self.common.base
    }

    fn base_mut(&mut self) -> &mut BasePlan<VM> {
        &mut self.common.base
    }

    fn common(&self) -> &CommonPlan<VM> {
        &self.common
    }

    fn gc_pause_start(&self, _scheduler: &GCWorkScheduler<VM>) {
        super::NO_EVAC.store(false, Ordering::SeqCst);
        let pause = self.current_pause().unwrap();

        super::SURVIVAL_RATIO_PREDICTOR.set_alloc_size(
            self.immix_space
                .block_allocation
                .total_young_allocation_in_bytes(),
        );
        self.immix_space.rc_eager_prepare(pause);

        for mutator in <VM as VMBinding>::VMActivePlan::mutators() {
            mutator.flush();
        }

        if pause == Pause::FinalMark {
            self.set_concurrent_marking_state(false);
        }
    }

    fn gc_pause_end(&self) {
        super::DISABLE_LASY_DEC_FOR_CURRENT_GC.store(false, Ordering::SeqCst);
        // self.immix_space.flush_page_resource();
        let pause = self.current_pause().unwrap();
        if pause == Pause::InitialMark {
            self.set_concurrent_marking_state(true);
        }
        self.previous_pause.store(Some(pause), Ordering::SeqCst);
        self.current_pause.store(None, Ordering::SeqCst);
        LAZY_SWEEPING_JOBS.write().swap();
        if super::LAZY_DECREMENTS {
            let perform_cycle_collection =
                self.get_available_pages() < super::CYCLE_TRIGGER_THRESHOLD;
            self.hint_cycle_gc
                .store(perform_cycle_collection, Ordering::SeqCst);
            self.hint_emergency_gc.store(false, Ordering::SeqCst);
            self.perform_cycle_collection.store(false, Ordering::SeqCst);
        }
        self.avail_pages_at_end_of_last_gc
            .store(self.get_available_pages(), Ordering::SeqCst);
        HEAP_AFTER_GC.store(self.get_reserved_pages(), Ordering::SeqCst);
    }

    fn end_of_gc(&mut self, _tls: VMWorkerThread) {}

    fn no_mutator_prepare_release(&self) -> bool {
        true
    }

    fn no_worker_prepare(&self) -> bool {
        true
    }

    fn fast_worker_release(&self) -> bool {
        true
    }
}

impl<VM: VMBinding> LXR<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Box<Self> {
        let immix_specs = metadata::extract_side_metadata(&[
            MetadataSpec::OnSide(RC_TABLE),
            MetadataSpec::OnSide(
                *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
                    .as_spec()
                    .extract_side_spec(),
            ),
            MetadataSpec::OnSide(Block::DEFRAG_STATE_TABLE),
        ]);
        let global_side_metadata_specs = SideMetadataContext::new_global_specs(&immix_specs);
        let mut plan_args = CreateSpecificPlanArgs {
            global_args: args,
            constraints: &LXR_CONSTRAINTS,
            global_side_metadata_specs,
        };
        let immix_space = ImmixSpace::new(
            plan_args.get_mature_space_args("immix", true, false, VMRequest::discontiguous()),
            ImmixSpaceArgs {
                never_move_objects: false,
                mixed_age: false,
            },
        );
        let mut lxr = Box::new(LXR {
            immix_space,
            common: CommonPlan::new(plan_args),
            perform_cycle_collection: AtomicBool::new(false),
            hint_cycle_gc: AtomicBool::new(false),
            hint_emergency_gc: AtomicBool::new(false),
            current_pause: Atomic::new(None),
            previous_pause: Atomic::new(None),
            last_gc_was_defrag: AtomicBool::new(false),
            avail_pages_at_end_of_last_gc: AtomicUsize::new(0),
            zeroing_packets_scheduled: AtomicBool::new(false),
            decide_cycle_collection: (Mutex::new(true), Condvar::new()),
            in_concurrent_marking: AtomicBool::new(false),
            prev_roots: Default::default(),
            curr_roots: Default::default(),
            rc: RefCountHelper::NEW,
        });

        lxr.gc_init();

        lxr.verify_side_metadata_sanity();

        lxr
    }

    pub fn cm_enabled(&self) -> bool {
        self.immix_space.cm_enabled
    }

    pub fn cm_in_progress(&self) -> bool {
        self.in_concurrent_marking.load(Ordering::Relaxed)
    }

    fn next_gc_is_emergency_gc(
        &self,
        total_pages: usize,
        mature_space_pages: usize,
        emergency_threshold: usize,
    ) -> bool {
        let min_avail_pages = usize::min(total_pages * emergency_threshold / 100, 1 << 30 >> 12);
        total_pages < min_avail_pages + mature_space_pages
    }

    fn next_gc_is_cycle_gc(
        &self,
        _total_pages: usize,
        mature_space_pages: usize,
        _cm_threshold: usize,
    ) -> bool {
        let live_mature_pages = super::MATURE_LIVE_PREDICTOR.live_pages() as usize;
        let garbage = mature_space_pages.saturating_sub(live_mature_pages);
        let total_pages = self.get_total_pages();
        !self.cm_in_progress()
            && (self.cm_enabled() && garbage * 100 >= super::TRACE_THRESHOLD * total_pages)
    }

    fn decide_next_gc_may_perform_cycle_collection(&self) {
        let (lock, cvar) = &self.decide_cycle_collection;
        let notify = || {
            let mut decide_cycle_collection = lock.lock().unwrap();
            *decide_cycle_collection = true;
            cvar.notify_one();
        };
        // Reset states
        self.hint_cycle_gc.store(false, Ordering::SeqCst);
        self.hint_emergency_gc.store(false, Ordering::SeqCst);
        let cm_threshold = super::TRACE_THRESHOLD;
        let emergency_threshold = super::RC_STOP_PERCENT;
        // Calculate mature space size
        let total_pages = self.get_total_pages();
        let mature_space_pages = {
            let released_los_pages = self.los().num_pages_released_lazy.load(Ordering::SeqCst);
            let pages_after_gc = HEAP_AFTER_GC
                .load(Ordering::SeqCst)
                .saturating_sub(
                    self.immix_space
                        .num_clean_blocks_released_lazy
                        .load(Ordering::SeqCst)
                        << Block::LOG_PAGES,
                )
                .saturating_sub(released_los_pages);
            pages_after_gc
        };
        // Decide next GC kind
        let hint_cycle_gc = self.next_gc_is_cycle_gc(total_pages, mature_space_pages, cm_threshold);
        let hint_emergency_gc =
            self.next_gc_is_emergency_gc(total_pages, mature_space_pages, emergency_threshold);
        // Update states
        self.hint_cycle_gc.store(hint_cycle_gc, Ordering::SeqCst);
        self.hint_emergency_gc
            .store(hint_emergency_gc, Ordering::SeqCst);
        // Eager mark-table zeroing
        if !cfg!(feature = "sanity") && hint_cycle_gc {
            self.schedule_mark_table_zeroing_tasks(None);
        }
        notify();
    }

    fn schedule_mark_table_zeroing_tasks(&self, pause: Option<Pause>) {
        if let Some(pause) = pause {
            assert!(pause == Pause::InitialMark || pause == Pause::Full);
            if self.zeroing_packets_scheduled.load(Ordering::SeqCst) {
                return;
            }
        }
        self.immix_space
            .schedule_mark_table_zeroing_tasks(WorkBucketStage::Unconstrained);
        self.zeroing_packets_scheduled.store(true, Ordering::SeqCst);
    }

    fn wait_for_decide_cycle_collection(&self) {
        let (lock, cvar) = &self.decide_cycle_collection;
        let mut decide_cycle_collection = lock.lock().unwrap();
        while !*decide_cycle_collection {
            decide_cycle_collection = cvar.wait(decide_cycle_collection).unwrap();
        }
        *decide_cycle_collection = false;
    }

    fn select_collection_kind(&self) -> Pause {
        self.wait_for_decide_cycle_collection();

        let emergency = self.base().global_state.is_emergency_collection();
        let user_triggered = self.base().global_state.is_user_triggered_collection();
        let cm_in_progress = self.cm_in_progress();
        let cm_packets_drained = super::concurrent_marking_packets_drained();
        let hint_cycle_gc = self.hint_cycle_gc.load(Ordering::SeqCst);
        let hint_emergency_gc = self.hint_emergency_gc.load(Ordering::SeqCst);
        // If CM is finished, do a final mark pause
        if cm_in_progress && cm_packets_drained {
            return Pause::FinalMark;
        }

        // Either final mark pause or full pause for emergency GC
        if emergency || user_triggered || hint_emergency_gc {
            return if cm_in_progress {
                Pause::FinalMark
            } else {
                Pause::Full
            };
        }

        // Should trigger CM?
        if hint_cycle_gc && !cm_in_progress {
            return if self.cm_enabled() {
                Pause::InitialMark
            } else {
                Pause::Full
            };
        } else {
            return Pause::RefCount;
        }
    }

    fn disable_unnecessary_buckets(&'static self, scheduler: &GCWorkScheduler<VM>, pause: Pause) {
        if pause == Pause::RefCount {
            scheduler.work_buckets[WorkBucketStage::Prepare].set_enabled(false);
        }
        if pause == Pause::RefCount || pause == Pause::InitialMark {
            scheduler.work_buckets[WorkBucketStage::Closure].set_enabled(false);
            scheduler.work_buckets[WorkBucketStage::WeakRefClosure].set_enabled(false);
            scheduler.work_buckets[WorkBucketStage::FinalRefClosure].set_enabled(false);
            scheduler.work_buckets[WorkBucketStage::PhantomRefClosure].set_enabled(false);
        }
        scheduler.work_buckets[WorkBucketStage::Concurrent].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::TPinningClosure].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::PinningRootsTrace].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::VMRefClosure].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::VMRefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::SoftRefClosure].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::CalculateForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::SecondRoots].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::RefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::FinalizableForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::Compact].set_enabled(false);
        if super::LAZY_DECREMENTS && pause != Pause::Full {
            scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].set_enabled(false);
        }
    }

    fn schedule_rc_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.disable_unnecessary_buckets(scheduler, Pause::RefCount);
        if self.cm_in_progress() {
            scheduler.pause_concurrent_marking_work_packets_during_gc();
        }
        type E<VM> = RCImmixCollectRootEdges<VM>;
        // Before start yielding, wrap all the roots from the previous GC with work-packets.
        self.process_prev_roots(scheduler);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add_prioritized(Box::new(StopMutators::<LXRGCWorkContext<E<VM>>>::new()));
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::RCProcessIncs].add(FastRCPrepare);
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<UnsupportedProcessEdges<VM>>>::new(self));
    }

    fn schedule_concurrent_marking_initial_pause(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.disable_unnecessary_buckets(scheduler, Pause::InitialMark);
        self.process_prev_roots(scheduler);
        scheduler.work_buckets[WorkBucketStage::Unconstrained].add_prioritized(Box::new(
            StopMutators::<LXRGCWorkContext<RCImmixCollectRootEdges<VM>>>::new(),
        ));
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<LXRGCWorkContext<UnsupportedProcessEdges<VM>>>::new(self));
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<UnsupportedProcessEdges<VM>>>::new(self));
    }

    fn schedule_concurrent_marking_final_pause(&'static self, scheduler: &GCWorkScheduler<VM>) {
        self.disable_unnecessary_buckets(scheduler, Pause::FinalMark);
        self.process_prev_roots(scheduler);
        scheduler.work_buckets[WorkBucketStage::Unconstrained].add_prioritized(Box::new(
            StopMutators::<LXRGCWorkContext<RCImmixCollectRootEdges<VM>>>::new(),
        ));

        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<LXRGCWorkContext<UnsupportedProcessEdges<VM>>>::new(self));
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<UnsupportedProcessEdges<VM>>>::new(self));
        scheduler.schedule_ref_proc_work::<LXRWeakRefWorkContext<VM>>(self);
    }

    fn schedule_emergency_full_heap_collection<E: ProcessEdgesWork<VM = VM>>(
        &'static self,
        scheduler: &GCWorkScheduler<VM>,
    ) {
        super::DISABLE_LASY_DEC_FOR_CURRENT_GC.store(true, Ordering::SeqCst);
        self.disable_unnecessary_buckets(scheduler, Pause::Full);
        // Before start yielding, wrap all the roots from the previous GC with work-packets.
        self.process_prev_roots(scheduler);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add_prioritized(Box::new(StopMutators::<LXRGCWorkContext<E>>::new()));
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<LXRGCWorkContext<UnsupportedProcessEdges<VM>>>::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<UnsupportedProcessEdges<VM>>>::new(self));
        scheduler.schedule_ref_proc_work::<LXRWeakRefWorkContext<VM>>(self);
    }

    fn process_prev_roots(&self, scheduler: &GCWorkScheduler<VM>) {
        let prev_roots = self.prev_roots.write().unwrap();
        let mut work_packets: Vec<Box<dyn GCWork<VM>>> = Vec::with_capacity(prev_roots.len());
        while let Some(decs) = prev_roots.pop() {
            work_packets.push(Box::new(ProcessDecs::new(
                decs,
                LazySweepingJobsCounter::new_decs(),
            )))
        }
        if work_packets.is_empty() {
            work_packets.push(Box::new(ProcessDecs::new(
                vec![],
                LazySweepingJobsCounter::new_decs(),
            )));
        }
        if super::LAZY_DECREMENTS {
            scheduler.postpone_all_prioritized(work_packets);
        } else {
            scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].bulk_add(work_packets);
        }
    }

    pub fn perform_cycle_collection(&self) -> bool {
        self.perform_cycle_collection.load(Ordering::SeqCst)
    }

    pub fn current_pause(&self) -> Option<Pause> {
        self.current_pause.load(Ordering::SeqCst)
    }

    pub fn previous_pause(&self) -> Option<Pause> {
        self.previous_pause.load(Ordering::SeqCst)
    }

    pub fn in_defrag(&self, o: ObjectReference) -> bool {
        Block::in_defrag_block::<VM>(o)
    }

    pub fn address_in_defrag(&self, a: Address) -> bool {
        self.immix_space.address_in_space(a) && Block::address_in_defrag_block(a)
    }

    pub fn mark(&self, o: ObjectReference) -> bool {
        if self.immix_space.in_space(o) {
            self.immix_space.attempt_mark(o)
        } else {
            self.common.los.attempt_mark(o)
        }
    }

    pub fn mark2(&self, o: ObjectReference, los: bool) -> bool {
        if !los {
            self.immix_space.attempt_mark(o)
        } else {
            self.common.los.attempt_mark(o)
        }
    }

    pub fn is_marked(&self, o: ObjectReference) -> bool {
        if self.immix_space.in_space(o) {
            self.immix_space.is_marked(o)
        } else {
            self.common.los.is_marked(o)
        }
    }

    pub const fn los(&self) -> &LargeObjectSpace<VM> {
        &self.common.los
    }

    fn on_lazy_decs_finished(&self, c: LazySweepingJobsCounter) {
        self.immix_space.schedule_rc_block_sweeping_tasks(c);
    }

    fn on_lazy_sweeping_finished(&self) {
        self.immix_space.flush_page_resource();
        // Update counters
        if !super::LAZY_DECREMENTS {
            HEAP_AFTER_GC.store(self.get_used_pages(), Ordering::SeqCst);
        }
        self.decide_next_gc_may_perform_cycle_collection();
    }

    fn gc_init(&mut self) {
        self.immix_space.cm_enabled = !cfg!(feature = "lxr_no_cm");
        self.immix_space.rc_enabled = true;
        self.common.los.rc_enabled = true;
        unsafe {
            let me = &*(self as *const Self);
            self.immix_space.block_allocation.lxr = Some(me);
            self.common.los.lxr = Some(me);
        }
        let mut lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.write();
        lazy_sweeping_jobs.swap();
        let lxr_ptr = self as *const Self as usize;
        lazy_sweeping_jobs.end_of_decs = Some(Box::new(move |c| {
            let lxr = unsafe { &*(lxr_ptr as *const Self) };
            lxr.on_lazy_decs_finished(c);
        }));
        lazy_sweeping_jobs.end_of_lazy = Some(Box::new(move || {
            let lxr = unsafe { &*(lxr_ptr as *const Self) };
            lxr.on_lazy_sweeping_finished();
        }));
    }

    fn set_concurrent_marking_state(&self, active: bool) {
        <VM as VMBinding>::VMCollection::set_concurrent_marking_state(active);
        self.in_concurrent_marking.store(active, Ordering::SeqCst);
    }

    pub fn options(&self) -> &Options {
        &self.common.base.options
    }
}
