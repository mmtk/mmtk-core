use super::gc_work::{LXRGCWorkContext, LXRWeakRefWorkContext, ReleaseLOSNursery};
use super::mutator::ALLOCATOR_MAPPING;
use super::rc::{ProcessDecs, RCImmixCollectRootEdges};
use super::remset::FlushMatureEvacRemsets;
use crate::mmtk::VM_MAP;
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
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::{PageResource, SpaceStats, VMRequest};
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::MetadataSpec;
use crate::util::options::{GCTriggerSelector, Options};
use crate::util::rc::{RefCountHelper, RC_TABLE};
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::{metadata, Address, ObjectReference};
use crate::vm::ActivePlan;
use crate::vm::{Collection, ObjectModel, VMBinding};
use crate::{policy::immix::ImmixSpace, util::opaque_pointer::VMWorkerThread};
use crate::{BarrierSelector, LazySweepingJobsCounter};
use atomic::{Atomic, Ordering};
use crossbeam::queue::SegQueue;
use enum_map::EnumMap;
use spin::Lazy;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Condvar, Mutex, RwLock};
use std::time::SystemTime;

const LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER: usize = 1;

static INCS_TRIGGERED: AtomicBool = AtomicBool::new(false);
static ALLOC_TRIGGERED: AtomicBool = AtomicBool::new(false);
static SURVIVAL_TRIGGERED: AtomicBool = AtomicBool::new(false);
static HEAP_AFTER_GC: AtomicUsize = AtomicUsize::new(0);

static RC_PAUSES_BEFORE_SATB: AtomicUsize = AtomicUsize::new(0);
static MAX_RC_PAUSES_BEFORE_SATB: AtomicUsize = AtomicUsize::new(128);

use bytemuck::NoUninit;
use mmtk_macros::{HasSpaces, PlanTraceObject};

#[derive(Debug, Clone, Copy, NoUninit)]
#[repr(u8)]
enum GCCause {
    Unknown,
    FullHeap,
    Emergency,
    UserTriggered,
    FixedNursery,
    Survival,
    Increments,
    ImmixSpaceFull,
    FinalMark,
}

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
    nursery_blocks: usize,
    young_alloc_trigger: usize,
    avail_pages_at_end_of_last_gc: AtomicUsize,
    zeroing_packets_scheduled: AtomicBool,
    decide_cycle_collection: (Mutex<bool>, Condvar),
    in_concurrent_marking: AtomicBool,
    pub prev_roots: RwLock<SegQueue<Vec<ObjectReference>>>,
    pub curr_roots: RwLock<SegQueue<Vec<ObjectReference>>>,
    pub(super) barrier_decs: AtomicUsize,
    pub rc: RefCountHelper<VM>,
    gc_cause: Atomic<GCCause>,
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
            self.gc_cause.store(GCCause::FullHeap, Ordering::Relaxed);
            return true;
        }
        // SATB is finished
        if self.cm_in_progress() && crate::concurrent_marking_packets_drained() {
            self.gc_cause.store(GCCause::FinalMark, Ordering::Relaxed);
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
        if !cfg!(feature = "lxr_no_survival_trigger") {
            if predicted_survival_mb >= crate::args().max_survival_mb {
                SURVIVAL_TRIGGERED.store(true, Ordering::Relaxed);
                self.gc_cause.store(GCCause::Survival, Ordering::Relaxed);
                return true;
            }
        }
        if !self.immix_space.common().contiguous {
            let available_to_space = (self.immix_space.pr.available_pages()
                + (VM_MAP.available_chunks() << (LOG_BYTES_IN_CHUNK - LOG_BYTES_IN_PAGE as usize)))
                >> (LOG_BYTES_IN_MBYTE - LOG_BYTES_IN_PAGE);
            if predicted_survival_mb >= available_to_space {
                self.gc_cause
                    .store(GCCause::ImmixSpaceFull, Ordering::Relaxed);
                return true;
            }
        }
        // inc limits
        if crate::args()
            .incs_limit
            .map(|x| self.rc.inc_buffer_size() >= x)
            .unwrap_or(false)
        {
            self.gc_cause.store(GCCause::Increments, Ordering::Relaxed);
            return true;
        }
        // clean young blocks limits
        if self.immix_space.block_allocation.clean_nursery_blocks() >= self.nursery_blocks {
            self.gc_cause
                .store(GCCause::FixedNursery, Ordering::Relaxed);
            return true;
        }
        // total young alloc limits (including clean and recycled allocation)
        if self
            .immix_space
            .block_allocation
            .total_young_allocation_in_bytes()
            >= self.young_alloc_trigger
        {
            self.gc_cause
                .store(GCCause::FixedNursery, Ordering::Relaxed);
            return true;
        }
        // Concurrent tracing finished
        // if self.cm_in_progress()
        //     && crate::concurrent_marking_packets_drained()
        // {
        //     return true;
        // }
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
        #[cfg(feature = "nogc_no_zeroing")]
        if true {
            unreachable!();
        }
        if !crate::LazySweepingJobs::all_finished() {
            gc_log!([1] "WARNING: LXR Lazy Sweeping Not Finished");
            crate::counters()
                .gc_with_unfinished_lazy_jobs
                .fetch_add(1, Ordering::Relaxed);
        }
        let pause = self.select_collection_kind();
        self.update_stats_after_gc_decided(pause);
        // Wait for concurrent packets
        if self.cm_in_progress() && pause == Pause::RefCount {
            crate::counters()
                .rc_during_satb
                .fetch_add(1, Ordering::SeqCst);
        }
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
        // Dump stats
        self.log_gc_start(pause);
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
        if cfg!(not(feature = "fragmentation_analysis"))
            && (pause == Pause::Full || pause == Pause::FinalMark)
        {
            #[cfg(feature = "sanity")]
            scheduler.work_buckets[WorkBucketStage::Final].add(ScheduleSanityGC::<Self>::new(self));
        }

        #[cfg(feature = "sanity")]
        if cfg!(feature = "fragmentation_analysis")
            && pause == Pause::RefCount
            && crate::frag_exp_enabled()
        {
            scheduler.work_buckets[WorkBucketStage::Final].add(ScheduleSanityGC::<Self>::new(self));
        }
    }

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector> {
        &*ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        #[cfg(feature = "measure_trace_rate")]
        if crate::verbose(3) {
            super::cm::dump_trace_rate();
        }
        let pause = self.current_pause().unwrap();
        crate::stat(|s| {
            if pause == Pause::RefCount {
                s.rc_pauses += 1
            }
        });
        if pause == Pause::FinalMark || pause == Pause::Full {
            self.common.los.is_end_of_satb_or_full_gc = true;
            // release nursery memory before mature evacuation, to reduce the chance of to-space overflow.
            self.immix_space.scheduler().work_buckets[WorkBucketStage::Unconstrained]
                .add(ReleaseLOSNursery);
        }
        self.common
            .prepare(tls, pause == Pause::Full || pause == Pause::InitialMark);
        if crate::args::RC_MATURE_EVACUATION && (pause == Pause::FinalMark || pause == Pause::Full)
        {
            self.immix_space.process_mature_evacuation_remset();
            self.immix_space.scheduler().work_buckets[WorkBucketStage::RCEvacuateMature]
                .add(FlushMatureEvacRemsets);
        }
        self.immix_space.prepare_rc(pause);
        // if pause == Pause::FinalMark {
        //     self.dump_heap_usage(false);
        // }
    }

    fn release(&mut self, tls: VMWorkerThread) {
        #[cfg(feature = "measure_rc_rate")]
        if crate::verbose(3) {
            super::rc::dump_rc_rate();
        }
        let _new_ratio = super::SURVIVAL_RATIO_PREDICTOR.update_ratio();
        let pause = self.current_pause().unwrap();
        if pause == Pause::FinalMark || pause == Pause::Full {
            #[cfg(feature = "lxr_release_stage_timer")]
            gc_log!([3]
                "    - ({:.3}ms) update_weak_processor start",
                crate::gc_start_time_ms(),
            );
            VM::VMCollection::update_weak_processor(false);
        }
        let perform_class_unloading = self.current_gc_should_perform_class_unloading();
        if perform_class_unloading {
            gc_log!([3] "    - class unloading");
        }
        #[cfg(feature = "lxr_release_stage_timer")]
        gc_log!([3]
            "    - ({:.3}ms) vm_release start",
            crate::gc_start_time_ms(),
        );
        let t = std::time::SystemTime::now();
        <VM as VMBinding>::VMCollection::vm_release(perform_class_unloading);
        let elapsed = t.elapsed().unwrap().as_micros() as f64;
        if perform_class_unloading {
            gc_log!([3] "    - class unloading finished in {:.3} ms", elapsed / 1000.0);
        }
        self.common.los.is_end_of_satb_or_full_gc = false;
        #[cfg(feature = "lxr_release_stage_timer")]
        gc_log!([3]
            "    - ({:.3}ms) los release start",
            crate::gc_start_time_ms(),
        );
        self.common
            .release(tls, pause == Pause::Full || pause == Pause::FinalMark);
        #[cfg(feature = "lxr_release_stage_timer")]
        gc_log!([3]
            "    - ({:.3}ms) ix release start",
            crate::gc_start_time_ms(),
        );
        self.immix_space.release_rc(pause);
        self.update_fixed_alloc_trigger();
        self.update_fragmentation_analysis_experiment();
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
        Block::update_global_phase_epoch(&self.immix_space);
    }

    fn get_collection_reserved_pages(&self) -> usize {
        let survival = if !cfg!(feature = "lxr_no_srv_copy_reserve") {
            let predicted_survival = (self.immix_space.block_allocation.clean_nursery_mb() as f64
                * super::SURVIVAL_RATIO_PREDICTOR.ratio())
                as usize;
            predicted_survival << LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER
        } else {
            0
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
        Block::update_global_phase_epoch(&self.immix_space);
        self.dump_heap_usage(true);
        crate::NO_EVAC.store(false, Ordering::SeqCst);
        let pause = self.current_pause().unwrap();

        super::SURVIVAL_RATIO_PREDICTOR.set_alloc_size(
            self.immix_space
                .block_allocation
                .total_young_allocation_in_bytes(),
        );
        super::SURVIVAL_RATIO_PREDICTOR.pause_start.start();
        self.immix_space.rc_eager_prepare(pause);

        if pause == Pause::FinalMark {
            // Flush barrier buffers before FinishConcurrentWork bucket
            for mutator in <VM as VMBinding>::VMActivePlan::mutators() {
                mutator.flush();
            }
            self.set_concurrent_marking_state(false);
            if cfg!(feature = "satb_timer") {
                let t = crate::SATB_START.elapsed().as_nanos();
                crate::counters().satb_nanos.fetch_add(t, Ordering::SeqCst);
            }
        } else if cfg!(feature = "satb_timer") && pause == Pause::RefCount && self.cm_in_progress()
        {
            let t = crate::SATB_START.elapsed().as_nanos();
            crate::counters().satb_nanos.fetch_add(t, Ordering::SeqCst);
        }

        if cfg!(feature = "decs_counter") {
            gc_log!([3] "POSTPONED {} DELETED OBJS FOR DECREMENT", self.barrier_decs.load(Ordering::SeqCst));
            self.barrier_decs.store(0, Ordering::SeqCst);
        }
    }

    fn gc_pause_end(&self) {
        crate::DISABLE_LASY_DEC_FOR_CURRENT_GC.store(false, Ordering::SeqCst);
        // self.immix_space.flush_page_resource();
        let pause = self.current_pause().unwrap();
        if pause == Pause::InitialMark {
            self.set_concurrent_marking_state(true);
            crate::REMSET_RECORDING.store(true, Ordering::SeqCst);
            if cfg!(feature = "satb_timer") {
                crate::SATB_START.start();
            }
        } else if cfg!(feature = "satb_timer") && pause == Pause::RefCount && self.cm_in_progress()
        {
            crate::SATB_START.start();
        }
        // if pause == Pause::RefCount || pause == Pause::InitialMark {
        //     self.resize_nursery();
        // }
        self.previous_pause.store(Some(pause), Ordering::SeqCst);
        self.current_pause.store(None, Ordering::SeqCst);
        crate::LAZY_SWEEPING_JOBS.write().swap();
        if crate::args::LAZY_DECREMENTS {
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
        self.dump_heap_usage(false);
        if cfg!(feature = "object_size_distribution") {
            if pause == Pause::FinalMark || pause == Pause::Full {
                crate::dump_and_reset_obj_dist("Static", &mut crate::OBJ_COUNT.lock().unwrap());
            }
        }
        if cfg!(feature = "lxr_satb_live_bytes_counter") {
            if pause == Pause::FinalMark || pause == Pause::Full {
                crate::report_and_reset_live_bytes();
            }
        }
        gc_log!([3] " - released young blocks since gc start {}({}M)", self.immix_space.num_clean_blocks_released_young.load(Ordering::Relaxed), self.immix_space.num_clean_blocks_released_young.load(Ordering::Relaxed) >> (LOG_BYTES_IN_MBYTE as usize - Block::LOG_BYTES));

        if cfg!(feature = "fragmentation_analysis") && crate::frag_exp_enabled() {
            self.dump_memory(pause);
        }
    }

    fn end_of_gc(&mut self, _tls: VMWorkerThread) {}

    #[cfg(feature = "nogc_no_zeroing")]
    fn handle_user_collection_request(&self, _tls: crate::util::VMMutatorThread, _force: bool) {
        println!("Warning: User attempted a collection request. The request is ignored.");
    }

    fn no_mutator_prepare_release(&self) -> bool {
        true
    }

    fn no_worker_prepare(&self) -> bool {
        true
    }

    fn fast_worker_release(&self) -> bool {
        true
    }

    fn should_process_reference(
        &self,
        reference: ObjectReference,
        referent: ObjectReference,
    ) -> bool {
        true
    }

    fn discover_reference(&self, reference: ObjectReference, referent: ObjectReference) {
        // Keep weak references and referents alive during SATB.
        // They can only be swept by mature sweeping.
        let _ = self.rc.inc(reference);
        let _ = self.rc.inc(referent);
    }

    fn current_gc_should_prepare_for_class_unloading(&self) -> bool {
        let pause = self.current_pause().unwrap();
        pause == Pause::InitialMark || pause == Pause::Full
    }

    fn current_gc_should_perform_class_unloading(&self) -> bool {
        let pause = self.current_pause().unwrap();
        pause == Pause::FinalMark || pause == Pause::Full
    }

    fn requires_weak_root_scanning(&self) -> bool {
        // Collect weak roots and keep them alive across RC pauses.
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
        let options = args.options.clone();
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
            nursery_blocks: crate::args().nursery_blocks.unwrap_or(usize::MAX),
            young_alloc_trigger: crate::args()
                .young_limit_mb
                .map(|x| x << LOG_BYTES_IN_MBYTE)
                .unwrap_or(usize::MAX),
            avail_pages_at_end_of_last_gc: AtomicUsize::new(0),
            zeroing_packets_scheduled: AtomicBool::new(false),
            decide_cycle_collection: (Mutex::new(true), Condvar::new()),
            in_concurrent_marking: AtomicBool::new(false),
            prev_roots: Default::default(),
            curr_roots: Default::default(),
            rc: RefCountHelper::NEW,
            gc_cause: Atomic::new(GCCause::Unknown),
            barrier_decs: AtomicUsize::default(),
        });

        lxr.update_fixed_alloc_trigger();

        lxr.gc_init(&options);

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
        total_pages: usize,
        mature_space_pages: usize,
        cm_threshold: usize,
        pause: Pause,
    ) -> bool {
        if cfg!(feature = "lxr_simple_satb_trigger") {
            // Do cycle collection if mature space is over 60% of the heap
            return if !self.cm_in_progress() {
                if self.cm_enabled() {
                    mature_space_pages * 100 >= cm_threshold * total_pages
                } else {
                    false
                }
            } else {
                false
            };
        }
        if pause == Pause::FinalMark || pause == Pause::Full {
            let live_mature_pages = super::MATURE_LIVE_PREDICTOR.update(mature_space_pages);
            gc_log!([3] " - predicted live mature pages: {}", live_mature_pages)
        }
        let live_mature_pages = super::MATURE_LIVE_PREDICTOR.live_pages() as usize;
        let garbage = mature_space_pages.saturating_sub(live_mature_pages);
        let total_pages = self.get_total_pages();
        let stop_pages = total_pages * crate::args().rc_stop_percent / 100;
        let available_pages = total_pages.saturating_sub(mature_space_pages);
        gc_log!(
            " - total_pages={} stop_pages={} mature_space_pages={} available_pages={}",
            total_pages,
            stop_pages,
            mature_space_pages,
            available_pages
        );
        !cfg!(feature = "lxr_fixed_satb_trigger")
            && !self.cm_in_progress()
            && (self.cm_enabled()
                && garbage * 100 >= crate::args().trace_threshold as usize * total_pages)
    }

    fn decide_next_gc_may_perform_cycle_collection(&self, pause: Pause) {
        let (lock, cvar) = &self.decide_cycle_collection;
        let notify = || {
            let mut decide_cycle_collection = lock.lock().unwrap();
            *decide_cycle_collection = true;
            cvar.notify_one();
        };
        // Reset states
        self.hint_cycle_gc.store(false, Ordering::SeqCst);
        self.hint_emergency_gc.store(false, Ordering::SeqCst);
        let cm_threshold = crate::args().trace_threshold;
        let emergency_threshold = crate::args().rc_stop_percent;
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
        let hint_cycle_gc =
            self.next_gc_is_cycle_gc(total_pages, mature_space_pages, cm_threshold, pause);
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

    fn log_gc_start(&self, pause: Pause) {
        let gc_cause = if self.base().global_state.is_emergency_collection() {
            GCCause::Emergency
        } else if self.base().global_state.is_user_triggered_collection() {
            GCCause::UserTriggered
        } else {
            self.gc_cause.load(Ordering::SeqCst)
        };
        let epoch = crate::GC_EPOCH.load(Ordering::SeqCst);
        let alloc_ix = self
            .immix_space
            .block_allocation
            .total_young_allocation_in_bytes();
        let alloc_los = self.los().young_alloc_size.load(Ordering::Relaxed);
        let alloc_total = alloc_los + alloc_ix;
        gc_log!([2]
            "GC({}) {:?} start. cause={:?} incs={} young-alloc={}M young-alloc-ix={}M young-clean-blocks={}({}M)  young-alloc-los={}M",
            epoch,
            pause,
            gc_cause,
            self.rc.inc_buffer_size(),
            alloc_total >> LOG_BYTES_IN_MBYTE,
            alloc_ix >> LOG_BYTES_IN_MBYTE,
            self.immix_space.block_allocation.clean_nursery_blocks(),
            self.immix_space.block_allocation.clean_nursery_blocks() << Block::LOG_BYTES >> LOG_BYTES_IN_MBYTE,
            alloc_los >> LOG_BYTES_IN_MBYTE,
        );

        // Counters and logs for special GC triggers

        if cfg!(feature = "lxr_fixed_satb_trigger") {
            let hours = |hrs: usize| std::time::Duration::from_secs((60 * 60 * hrs) as u64);
            let date230505 = std::time::SystemTime::UNIX_EPOCH + hours(467575);
            let d = SystemTime::now().duration_since(date230505).unwrap();
            let hrs = (d.as_secs() / 3600) % 24;
            let new_value: usize = match hrs {
                _ if hrs < 12 => 32,
                _ => 16,
            };
            if new_value != MAX_RC_PAUSES_BEFORE_SATB.load(Ordering::Relaxed) {
                gc_log!([1] "===>>> Update SATB Trigger: {:?} <<<===", new_value);
                MAX_RC_PAUSES_BEFORE_SATB.store(new_value, Ordering::Relaxed);
            }
        }
    }

    fn select_collection_kind(&self) -> Pause {
        self.wait_for_decide_cycle_collection();

        let emergency = self.base().global_state.is_emergency_collection();
        let user_triggered = self.base().global_state.is_user_triggered_collection();
        let cm_in_progress = self.cm_in_progress();
        let cm_packets_drained = crate::concurrent_marking_packets_drained();
        let hint_cycle_gc = self.hint_cycle_gc.load(Ordering::SeqCst);
        let hint_emergency_gc = self.hint_emergency_gc.load(Ordering::SeqCst);
        // Dump stats
        if crate::verbose(3) {
            if self.cm_enabled() && cm_in_progress {
                if cm_packets_drained {
                    gc_log!([3] "SATB: Concurrent marking is done");
                } else {
                    gc_log!([3] "SATB: Concurrent marking is NOT done. {} packets remaining", crate::NUM_CONCURRENT_TRACING_PACKETS.load(Ordering::SeqCst));
                }
            }
            gc_log!([3] "Hint: cycle = {}, emergency = {}", hint_cycle_gc, hint_emergency_gc);
            gc_log!([3]
                "Stat: emergency = {}, user_triggered = {}, cm_in_progress = {}, cm_packets = {}, cm_enabled = {}",
                emergency, user_triggered, cm_in_progress,
                crate::NUM_CONCURRENT_TRACING_PACKETS.load(Ordering::SeqCst),
                self.cm_enabled(),
            );
        }

        // If CM is finished, do a final mark pause
        if cm_in_progress
            && (cfg!(feature = "measure_trace_rate")
                || crate::args::NO_RC_PAUSES_DURING_CONCURRENT_MARKING
                || cm_packets_drained)
        {
            return Pause::FinalMark;
        }

        // Either final mark pause or full pause for emergency GC
        if emergency
            || (user_triggered && !cfg!(feature = "lxr_abort_on_trace"))
            || hint_emergency_gc
        {
            return if cm_in_progress {
                Pause::FinalMark
            } else {
                Pause::Full
            };
        }

        // Should trigger CM?
        if cfg!(feature = "lxr_fixed_satb_trigger") {
            if RC_PAUSES_BEFORE_SATB.load(Ordering::Relaxed) + 1
                >= MAX_RC_PAUSES_BEFORE_SATB.load(Ordering::Relaxed)
                && !cm_in_progress
            {
                return if self.cm_enabled() {
                    Pause::InitialMark
                } else {
                    Pause::Full
                };
            } else {
                return Pause::RefCount;
            }
        }
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

    fn update_stats_after_gc_decided(&self, pause: Pause) {
        let emergency = self.base().global_state.is_emergency_collection();
        let cm_drained = crate::concurrent_marking_packets_drained();
        let counters = crate::counters();
        // cm state counters
        if pause == Pause::FinalMark && !cm_drained {
            counters.cm_early_quit.fetch_add(1, Ordering::Relaxed);
        }
        // gc type counters
        if emergency {
            counters.emergency.fetch_add(1, Ordering::Relaxed);
        }
        match pause {
            Pause::RefCount => counters.rc.fetch_add(1, Ordering::Relaxed),
            Pause::InitialMark => counters.initial_mark.fetch_add(1, Ordering::Relaxed),
            Pause::FinalMark => counters.final_mark.fetch_add(1, Ordering::Relaxed),
            _ => counters.full.fetch_add(1, Ordering::Relaxed),
        };
        // gc trigger counters
        if SURVIVAL_TRIGGERED.load(Ordering::Relaxed) {
            counters.survival_triggerd.fetch_add(1, Ordering::Relaxed);
        } else if INCS_TRIGGERED.load(Ordering::Relaxed) {
            counters.incs_triggerd.fetch_add(1, Ordering::Relaxed);
        } else if ALLOC_TRIGGERED.load(Ordering::Relaxed) {
            counters.alloc_triggerd.fetch_add(1, Ordering::Relaxed);
        } else {
            counters.overflow_triggerd.fetch_add(1, Ordering::Relaxed);
        }
        SURVIVAL_TRIGGERED.store(false, Ordering::Relaxed);
        INCS_TRIGGERED.store(false, Ordering::Relaxed);
        ALLOC_TRIGGERED.store(false, Ordering::Relaxed);
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
        scheduler.work_buckets[WorkBucketStage::FixRelocations].set_enabled(false);
        if crate::args::LAZY_DECREMENTS
            && pause != Pause::Full
            && !cfg!(feature = "fragmentation_analysis")
        {
            scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].set_enabled(false);
        }
    }

    fn schedule_rc_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        if cfg!(feature = "lxr_fixed_satb_trigger") {
            RC_PAUSES_BEFORE_SATB.fetch_add(1, Ordering::Relaxed);
        }
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

    fn dump_memory(&self, pause: Pause) {
        if pause != Pause::Full {
            // println!("\n\n\n@@ FRAGMENTATION DISTRIBUTION - Full\n\n");
            return;
        }
        eprintln!("\n\n\n@@ FRAGMENTATION DISTRIBUTION - {:?}\n", pause);
        eprintln!("heap-size: {}", self.get_total_pages() << 12);
        self.immix_space.dump_memory(self);
        self.los().dump_memory(self);
        eprintln!("\n@@ FRAGMENTATION DISTRIBUTION - {:?} End\n\n", pause);
        // }
    }

    fn schedule_concurrent_marking_initial_pause(&'static self, scheduler: &GCWorkScheduler<VM>) {
        if cfg!(feature = "lxr_abort_on_trace") {
            panic!("ERROR: OutOfMemory");
        }
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
        if cfg!(feature = "lxr_abort_on_trace") {
            panic!("ERROR: OutOfMemory");
        }
        if cfg!(feature = "lxr_fixed_satb_trigger") {
            RC_PAUSES_BEFORE_SATB.store(0, Ordering::Relaxed);
        }
        self.disable_unnecessary_buckets(scheduler, Pause::FinalMark);
        if self.cm_in_progress() {
            crate::MOVE_CONCURRENT_MARKING_TO_STW.store(true, Ordering::SeqCst);
        }
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
        if cfg!(feature = "lxr_abort_on_trace") {
            panic!("ERROR: OutOfMemory");
        }
        if cfg!(feature = "lxr_fixed_satb_trigger") {
            RC_PAUSES_BEFORE_SATB.store(0, Ordering::Relaxed);
        }
        crate::DISABLE_LASY_DEC_FOR_CURRENT_GC.store(true, Ordering::SeqCst);
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
        let mut count = 0usize;
        let prev_roots = self.prev_roots.write().unwrap();
        let mut work_packets: Vec<Box<dyn GCWork<VM>>> = Vec::with_capacity(prev_roots.len());
        while let Some(decs) = prev_roots.pop() {
            count += decs.len();
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
        if crate::args::LAZY_DECREMENTS {
            debug_assert!(!crate::args::BARRIER_MEASUREMENT);
            scheduler.postpone_all_prioritized(work_packets);
        } else {
            scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].bulk_add(work_packets);
        }
        if cfg!(feature = "decs_counter") {
            gc_log!([3] "POSTPONED {} ROOTS FOR DECREMENT", count);
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
        gc_log!([2]
            " - lazy decs finished since-gc-start={:.3}ms",
            crate::gc_start_time_ms(),
        );
        self.immix_space.schedule_rc_block_sweeping_tasks(c);
    }

    fn on_lazy_sweeping_finished(&self) {
        let ix = &self.immix_space;
        self.immix_space.flush_page_resource();
        let released_blocks = ix.num_clean_blocks_released_lazy.load(Ordering::SeqCst);
        let released_los_pages = self.los().num_pages_released_lazy.load(Ordering::SeqCst);
        let total_released_bytes =
            (released_blocks << Block::LOG_BYTES) + (released_los_pages << LOG_BYTES_IN_PAGE);
        gc_log!([2]
            " - lazy jobs finished since-gc-start={:.3}ms, current-reserved-heap={}M({}M), released-blocks={}, released-los-pages={}, total-released={}",
            crate::gc_start_time_ms(),
            self.get_reserved_pages() / 256,
            self.get_total_pages() / 256,
            released_blocks,
            released_los_pages,
            if total_released_bytes < BYTES_IN_KBYTE {
                format!("{}B", total_released_bytes)
            } else if total_released_bytes < BYTES_IN_MBYTE {
                format!("{}K", total_released_bytes >> LOG_BYTES_IN_KBYTE)
            } else if total_released_bytes < (1 << 30) {
                format!("{}M", total_released_bytes >> LOG_BYTES_IN_MBYTE)
            } else {
                format!("{}G", total_released_bytes >> 30)
            }
        );
        gc_log!([3] " - released young blocks since gc start {}({}M)", ix.num_clean_blocks_released_young.load(Ordering::Relaxed), ix.num_clean_blocks_released_young.load(Ordering::Relaxed) >> (LOG_BYTES_IN_MBYTE as usize - Block::LOG_BYTES));
        gc_log!([3] " - released mature blocks {}({}M)", ix.num_clean_blocks_released_mature.load(Ordering::Relaxed), ix.num_clean_blocks_released_mature.load(Ordering::Relaxed) >> (LOG_BYTES_IN_MBYTE as usize - Block::LOG_BYTES));
        if cfg!(feature = "lxr_log_reclaim") {
            let rc_killed_ix = ix.rc_killed_bytes.load(Ordering::SeqCst);
            let rc_killed_los = self.los().rc_killed_bytes.load(Ordering::SeqCst);
            gc_log!([2]
                " - rc-killed={}({}M) rc-killed-ix={}({}M) rc-killed-los={}({}M)",
                (rc_killed_ix + rc_killed_los),
                (rc_killed_ix + rc_killed_los) / 1024 / 1024,
                rc_killed_ix,
                rc_killed_ix / 1024 / 1024,
                rc_killed_los,
                rc_killed_los / 1024 / 1024,
            );
        }
        gc_log!([2] " - num_clean_blocks_released_lazy = {}", ix.num_clean_blocks_released_lazy.load(Ordering::SeqCst));
        // Update counters
        if !crate::args::LAZY_DECREMENTS {
            HEAP_AFTER_GC.store(self.get_used_pages(), Ordering::SeqCst);
        }
        {
            let used_pages_after_gc = HEAP_AFTER_GC.load(Ordering::Relaxed);
            let lazy_released_pages =
                ix.num_clean_blocks_released_lazy.load(Ordering::Relaxed) << Block::LOG_PAGES;
            let x = used_pages_after_gc.saturating_sub(lazy_released_pages);
            let c = crate::counters();
            c.total_used_pages.fetch_add(x, Ordering::Relaxed);
            c.min_used_pages.fetch_min(x, Ordering::Relaxed);
            c.max_used_pages.fetch_max(x, Ordering::Relaxed);
        }
        let pause = match self.current_pause() {
            Some(p) => p,
            None => self.previous_pause().unwrap(),
        };
        self.decide_next_gc_may_perform_cycle_collection(pause);
    }

    fn gc_init(&mut self, options: &Options) {
        crate::args::validate_features(BarrierSelector::FieldBarrier, options);
        self.immix_space.cm_enabled = !cfg!(feature = "lxr_no_cm");
        self.immix_space.rc_enabled = true;
        self.common.los.rc_enabled = true;
        unsafe {
            let me = &*(self as *const Self);
            self.immix_space.block_allocation.lxr = Some(me);
            self.common.los.lxr = Some(me);
        }
        let mut lazy_sweeping_jobs = crate::LAZY_SWEEPING_JOBS.write();
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

        if let Some(nursery_ratio) = crate::args().nursery_ratio {
            let heap_size = match *options.gc_trigger {
                GCTriggerSelector::FixedHeapSize(x) => x,
                _ => unimplemented!(),
            };
            let total_blocks = heap_size >> Block::LOG_BYTES;
            let nursery_blocks = total_blocks / (nursery_ratio + 1);
            self.nursery_blocks = nursery_blocks;
        }
    }

    fn set_concurrent_marking_state(&self, active: bool) {
        <VM as VMBinding>::VMCollection::set_concurrent_marking_state(active);
        self.in_concurrent_marking.store(active, Ordering::SeqCst);
    }

    pub fn options(&self) -> &Options {
        &self.common.base.options
    }

    pub fn dump_heap_usage(&self, gc_start: bool) {
        gc_log!([3]
            " - reserved={}M (ix-{}M, los-{}M) collection_reserve={}M ix_avail={}M vmmap_avail={}M",
            self.get_reserved_pages() / 256,
            self.immix_space.reserved_pages() / 256,
            self.los().reserved_pages() / 256,
            self.get_collection_reserved_pages() / 256,
            self.immix_space.pr.available_pages() / 256,
            if self.immix_space.common().contiguous {
                0
            } else {
                (VM_MAP.available_chunks() << (LOG_BYTES_IN_CHUNK - LOG_BYTES_IN_PAGE as usize)) / 256
            },
        );
        gc_log!(
            " - ix-used-size: {}M, ix-virt-size: {}M, los-used-size: {}M,  los-virt-size: {}M",
            self.immix_space.pr.reserved_pages() * 4 / 1024,
            self.immix_space.pr.total_chunks.load(Ordering::SeqCst) * 4,
            self.los().pr.reserved_pages() * 4 / 1024,
            self.los().pr.total_chunks.load(Ordering::SeqCst) * 4,
        );
        crate::rust_mem_counter::dump(gc_start);
    }

    fn hours_since_monday_0am(&self) -> usize {
        let hours = |hrs: usize| std::time::Duration::from_secs((60 * 60 * hrs) as u64);
        let monday_20231106_aedt = std::time::SystemTime::UNIX_EPOCH + hours(471997);
        let duration = SystemTime::now()
            .duration_since(monday_20231106_aedt)
            .unwrap();
        let hrs = duration.as_secs() as usize / 3600;
        hrs % (7 * 24)
    }

    fn update_fragmentation_analysis_experiment(&mut self) {
        if !cfg!(feature = "periodic_fragmentation_analysis") {
            return;
        }
        let hrs = self.hours_since_monday_0am() % 3;
        if hrs < 1 {
            crate::FRAG_EXP_ENABLED.store(true, Ordering::SeqCst)
        } else {
            crate::FRAG_EXP_ENABLED.store(false, Ordering::SeqCst)
        }
    }

    fn update_fixed_alloc_trigger(&mut self) {
        if !cfg!(feature = "fixed_alloc_trigger_based_on_system_time")
            && !cfg!(feature = "fixed_clean_alloc_trigger_based_on_system_time")
        {
            return;
        }
        let hours = |hrs: usize| std::time::Duration::from_secs((60 * 60 * hrs) as u64);
        let date230505 = std::time::SystemTime::UNIX_EPOCH + hours(467575);
        let d = SystemTime::now().duration_since(date230505).unwrap();
        let hrs = (d.as_secs() / 3600) % 48;
        if cfg!(feature = "fixed_clean_alloc_trigger_based_on_system_time") {
            let new_value: usize = match hrs {
                _ if hrs < 8 => (4 << 30) >> Block::LOG_BYTES,    // 4G
                _ if hrs < 16 => (2 << 30) >> Block::LOG_BYTES,   // 2G
                _ if hrs < 24 => (1 << 30) >> Block::LOG_BYTES,   // 1G
                _ if hrs < 32 => (512 << 20) >> Block::LOG_BYTES, // 512M
                _ if hrs < 40 => (256 << 20) >> Block::LOG_BYTES, // 256M
                _ => (128 << 20) >> Block::LOG_BYTES,             // 128M
            };
            if new_value != self.nursery_blocks {
                gc_log!([1] "===>>> Update Fixed Clean Alloc Trigger: {:?} <<<===", new_value);
                self.nursery_blocks = new_value;
            }
        } else if cfg!(feature = "fixed_alloc_trigger_based_on_system_time") {
            let new_value: usize = match hrs {
                _ if hrs < 8 => 4096 << LOG_BYTES_IN_MBYTE,  // 4G
                _ if hrs < 16 => 2048 << LOG_BYTES_IN_MBYTE, // 2G
                _ if hrs < 24 => 1024 << LOG_BYTES_IN_MBYTE, // 1G
                _ if hrs < 32 => 512 << LOG_BYTES_IN_MBYTE,  // 512M
                _ if hrs < 40 => 256 << LOG_BYTES_IN_MBYTE,  // 256M
                _ => 128 << LOG_BYTES_IN_MBYTE,              // 128M
            };
            if new_value != self.young_alloc_trigger {
                gc_log!([1] "===>>> Update Fixed Alloc Trigger: {:?} <<<===", new_value);
                self.young_alloc_trigger = new_value;
            }
        }
    }
}
