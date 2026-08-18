use super::block_allocation::BlockAllocation;
use super::gc_work::nursery_sweeping::ReleaseLOSNursery;
use super::gc_work::prepare::FastRCPrepare;
use super::gc_work::rc::ProcessDecs;
use super::gc_work::LXRGCWorkContext;
use super::mature_evac::MatureEvacuationSet;
use super::mutator::ALLOCATOR_MAPPING;
use super::{LazySweepingJobsCounter, LAZY_SWEEPING_JOBS};
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::plan::global::CommonPlan;
use crate::plan::global::{BasePlan, CreateGeneralPlanArgs, CreateSpecificPlanArgs};
use crate::plan::lxr::gc_work::mature_sweeping::{RCSweepMatureAfterSATBLOS, SweepDeadCycles};
use crate::plan::lxr::gc_work::nursery_sweeping::SweepBlocksAfterDecs;
use crate::plan::lxr::gc_work::prepare::{ConcurrentChunkMetadataZeroing, PrepareChunksForFullGC};
use crate::plan::lxr::mature_evac::MatureEvecRemSet;
use crate::plan::AllocationSemantics;
use crate::plan::MutatorContext;
use crate::plan::Plan;
use crate::plan::PlanConstraints;
use crate::policy::immix::block::Block;
use crate::policy::immix::ImmixSpaceArgs;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(feature = "analysis")]
use crate::util::analysis::GcHookWork;
use crate::util::constants::*;
use crate::util::copy::*;
use crate::util::heap::{SpaceStats, VMRequest};
use crate::util::metadata::side_metadata::SideMetadataContext;
use crate::util::metadata::MetadataSpec;
use crate::util::rc::{RefCountHelper, RC_TABLE};
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::*;
use crate::util::{metadata, Address, ObjectReference};
use crate::vm::ActivePlan;
use crate::vm::{Collection, ObjectModel, VMBinding};
use crate::BarrierSelector;
use crate::{policy::immix::ImmixSpace, util::opaque_pointer::VMWorkerThread};
use crate::{scheduler::*, MMTK};
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
    avail_pages_at_end_of_last_gc: AtomicUsize,
    zeroing_packets_scheduled: AtomicBool,
    decide_cycle_collection: (Mutex<bool>, Condvar),
    in_concurrent_marking: AtomicBool,
    pub prev_roots: RwLock<SegQueue<Vec<ObjectReference>>>,
    pub curr_roots: RwLock<SegQueue<Vec<ObjectReference>>>,
    pub rc: RefCountHelper<VM>,
    block_allocation: BlockAllocation<VM>,
    pub(super) evac_set: MatureEvacuationSet,
    pub(super) mature_evac_remset: MatureEvecRemSet<VM>,
    /// Monotonic total of clean blocks released by deferred sweeping, ever.
    pub(super) num_clean_blocks_released_lazy: AtomicUsize,
    /// Value of the two totals above/in the LOS as of the end of the last pause, so that the
    /// reclamation attributable to that pause can be read off as a difference. See `gc_pause_end`.
    lazy_freed_blocks_at_pause_end: AtomicUsize,
    lazy_freed_los_pages_at_pause_end: AtomicUsize,
    /// The last cycle for which `on_lazy_sweeping_finished` took a decision, so that a cycle whose
    /// deferred work drains in more than one wave is only decided once.
    decided_epoch: AtomicUsize,
    pub(super) possibly_dead_mature_blocks: SegQueue<(Block, bool)>,
}

/// The static plan constraints for LXR: it uses a field-level write barrier with
/// log bits, enables reference counting, and moves objects unless both nursery and
/// mature evacuation have been disabled at build time.
pub static LXR_CONSTRAINTS: Lazy<PlanConstraints> = Lazy::new(|| PlanConstraints {
    moves_objects: super::NURSERY_EVACUATION || super::MATURE_EVACUATION,
    // Max immix object size is half of a block.
    max_non_los_default_alloc_bytes: crate::policy::immix::MAX_IMMIX_OBJECT_SIZE,
    barrier: BarrierSelector::FieldBarrier,
    needs_log_bit: true,
    needs_field_log_bit: true,
    rc_enabled: true,
    needs_prepare_mutator: false,
    ..PlanConstraints::default()
});

/// Identifies which policy owns an object, from the point of view of LXR's
/// reference counting and tracing.
///
/// LXR only reference counts the objects it allocates itself, which live either in
/// its Immix space or in the large object space. A plan also has an immortal space,
/// a non-moving space, and (under the `vm_space` feature) a space describing a boot
/// image supplied by the VM. Objects there have no reference count and no line
/// marks, so none of the RC or Immix metadata may be consulted for them, but they
/// still have to be traced because they can refer to reference counted objects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LXRSpace {
    /// LXR's Immix space: reference counted, line marked, and possibly evacuated.
    Immix,
    /// The large object space: reference counted, never moved.
    Los,
    /// A space owned by the common plan: the immortal space, the non-moving space,
    /// or the VM space. Not reference counted and never moved by LXR.
    Common,
}

impl<VM: VMBinding> LXR<VM> {
    /// Returns which policy owns `o`. See [`LXRSpace`].
    pub fn space_of(&self, o: ObjectReference) -> LXRSpace {
        if self.immix_space.in_space(o) {
            LXRSpace::Immix
        } else if self.common.los.in_space(o) {
            LXRSpace::Los
        } else {
            LXRSpace::Common
        }
    }

    /// Returns whether `o` carries a reference count, i.e. whether it lives in the
    /// Immix space or the large object space.
    pub fn is_rc_object(&self, o: ObjectReference) -> bool {
        self.space_of(o) != LXRSpace::Common
    }

    /// Whether any Immix line this object occupies currently reads as free.
    ///
    /// The hole finder decides a line is available from its reference counts, so a live
    /// object sitting on a line that reads as free means the allocator will hand that
    /// memory out and the mutator will overwrite the object in place. Bring-up check.
    pub fn object_occupies_free_line(&self, o: ObjectReference) -> bool {
        use crate::policy::immix::line::{Line, RCArray};
        use crate::util::linear_scan::Region;
        use crate::util::linear_scan::UnstraddlableRegion;
        if self.space_of(o) != LXRSpace::Immix {
            return false;
        }
        let start = VM::VMObjectModel::ref_to_object_start(o);
        let size = o.get_size::<VM>();
        let block = Block::containing(o);
        let rc_array = RCArray::of(block);
        let mut line = Line::from_unaligned_address(start);
        let end = Line::from_unaligned_address(start + (size - 1)).next();
        while line != end {
            if line.block() != block {
                break;
            }
            if rc_array.is_dead(line.get_index_within_block()) {
                return true;
            }
            line = line.next();
        }
        false
    }

    /// The root objects recorded so far in the current pause, without consuming them.
    ///
    /// Intended for bring-up verification: walking outwards from these reaches
    /// everything the collector believes is live, so any object found with a zero
    /// reference count is one that some store failed to count.
    pub fn snapshot_curr_roots(&self) -> Vec<ObjectReference> {
        let guard = self.curr_roots.read().unwrap();
        let mut batches = vec![];
        while let Some(b) = guard.pop() {
            batches.push(b);
        }
        let mut all = vec![];
        for b in batches {
            all.extend_from_slice(&b);
            guard.push(b);
        }
        all
    }
}

impl<VM: VMBinding> Plan for LXR<VM> {
    fn current_gc_may_move_object(&self) -> bool {
        super::NURSERY_EVACUATION || super::MATURE_EVACUATION
    }

    fn collection_required(&self, space_full: bool, _space: Option<SpaceStats<Self::VM>>) -> bool {
        // Spaces or heap full
        if self.base().collection_required(self, space_full) {
            return true;
        }
        // SATB is finished
        if self.concurrent_work_in_progress() && super::concurrent_marking_packets_drained() {
            return true;
        }
        // Bound the pause by bounding the work it has to do.
        //
        // Every pause drains the increment buffer the barrier has been filling, in
        // `RCProcessIncs`, and that is what a pause is made of: 12.9ms of a 14.2ms `RefCount`
        // pause and 5.5ms of a 7.9ms `FinalMark` on `tree_mutable`, with everything else in the
        // pause under a millisecond. Its size is set by how much the mutator has written since the
        // last pause, which the heap-occupancy trigger does not bound at all -- a mutation-heavy
        // program reaches the heap target having queued an unbounded number of increments.
        //
        // So collect on the buffer as well as on occupancy. This is `INC_BUFFER_LIMIT` from
        // upstream LXR, which has always been declared here (`rc::inc_buffer_size` is maintained
        // on the barrier's flush path and reset in `ImmixSpace::release_rc`) but never read.
        if let Some(limit) = super::inc_buffer_limit() {
            if self.rc.inc_buffer_size() >= limit {
                return true;
            }
        }
        // Survival limits
        let total_young_alloc_pages =
            self.block_allocation.total_young_allocation_in_bytes() >> LOG_BYTES_IN_MBYTE;
        // `promotion_ratio`, not `ratio`: this bound exists to cap the promotion work the next
        // pause will do, and every promotion costs that work whether or not the object moved.
        let predicted_survival_mb: usize = ((total_young_alloc_pages as f64
            * super::SURVIVAL_RATIO_PREDICTOR.promotion_ratio())
            as usize)
            << LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER;
        if predicted_survival_mb >= super::max_survival_mb() {
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

    fn last_collection_was_exhaustive(&self) -> bool {
        self.previous_pause.load(Ordering::SeqCst) == Some(Pause::Full)
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
        // Reset here, not in `prepare`: `RCProcessIncs` runs *before* `Prepare`, so a flag
        // cleared there is still set from the previous GC while this GC's increments run.
        super::GC_COUNT.fetch_add(1, Ordering::SeqCst);
        super::RELEASE_STARTED.store(false, Ordering::SeqCst);
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
            Pause::Full => self.schedule_emergency_full_heap_collection(scheduler),
            Pause::RefCount => self.schedule_rc_collection(scheduler),
            Pause::InitialMark => self.schedule_concurrent_marking_initial_pause(scheduler),
            Pause::FinalMark => self.schedule_concurrent_marking_final_pause(scheduler),
        }
        // NOTE: LXR does no VM weak-reference or finalizer processing. See the `VMRefClosure`
        // comment in `disable_unnecessary_buckets`, and the "For whoever builds LXR's
        // process_weak_refs" section of LXR_PROGRESS.md, which carries the `Trace` impl and
        // the sentinel scheduling this would need.
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
        &ALLOCATOR_MAPPING
    }

    fn prepare(&mut self, tls: VMWorkerThread) {
        let pause = self.current_pause().unwrap();
        if pause == Pause::FinalMark || pause == Pause::Full {
            self.common.los.is_end_of_satb_or_full_gc = true;
            // release nursery memory before mature evacuation, to reduce the chance of to-space overflow.
            self.immix_space.scheduler().work_buckets[WorkBucketStage::Unconstrained]
                .add(ReleaseLOSNursery);
        }
        // Only the pause that *begins* a mark cycle may clear the immortal/VM-space mark
        // bits. `FinalMark` closes the cycle that `InitialMark` opened, so clearing there
        // would throw away everything concurrent marking marked.
        let starts_mark_cycle = pause == Pause::Full || pause == Pause::InitialMark;
        self.common
            .prepare_ext(tls, starts_mark_cycle, starts_mark_cycle);
        if super::MATURE_EVACUATION && (pause == Pause::FinalMark || pause == Pause::Full) {
            self.process_mature_evacuation_remset();
        }
        if super::MATURE_EVACUATION && (pause == Pause::InitialMark || pause == Pause::Full) {
            // Select mature evacuation set
            self.schedule_defrag_selection_packets();
        }
        // `num_clean_blocks_released_lazy` is deliberately *not* reset here. It is a monotonic
        // total; a consumer takes the difference against the snapshot `gc_pause_end` records.
        // Zeroing it at the start of a pause used to discard the previous cycle's reclamation
        // before the decision that needed it had been taken. See `gc_pause_end`.
        self.immix_space.prepare_rc(pause);
        self.block_allocation
            .reset_block_mark_for_mutator_reused_blocks(pause);
    }

    fn release(&mut self, tls: VMWorkerThread) {
        super::RELEASE_STARTED.store(true, Ordering::SeqCst);
        let _new_ratio = super::SURVIVAL_RATIO_PREDICTOR.update_ratio();
        let pause = self.current_pause().unwrap();
        // Every pause, not just tracing ones, and before anything is reclaimed: the binding
        // uses this to drop registered finalizers, which LXR cannot run (see the `VMRefClosure`
        // comment in `disable_unnecessary_buckets`). Reference counting frees objects in
        // `RefCount` pauses too, so skipping those would leave a window in which an entry
        // outlives the object it names -- which is the crash this avoids.
        let stats = std::env::var_os("MMTK_LXR_STATS").is_some();
        let t0 = std::time::Instant::now();
        VM::VMCollection::update_weak_processor(true);
        let t_weak = t0.elapsed();
        <VM as VMBinding>::VMCollection::vm_release();
        let t_vm = t0.elapsed() - t_weak;
        self.common.los.is_end_of_satb_or_full_gc = false;
        self.common
            .release(tls, pause == Pause::Full || pause == Pause::FinalMark);
        let t_common = t0.elapsed() - t_weak - t_vm;
        if stats {
            eprintln!(
                "[lxr] release phases: weak={}us vm_release={}us common={}us",
                t_weak.as_micros(),
                t_vm.as_micros(),
                t_common.as_micros(),
            );
        }
        if std::env::var_os("MMTK_LXR_STATS").is_some() {
            // Per-object counts are USDT tracepoints rather than counters -- see
            // `lxr_inc_pushed`, `lxr_promote`, `lxr_slot_skipped` and friends in
            // `tools/tracing/README.md`. They used to be global atomics, but one of them sat
            // on the barrier's fast path, so every mutator thread contended a single cache
            // line on every reference store and no timing taken here meant anything.
            eprintln!(
                "[lxr] gc#{} release: pause={:?} reserved_pages={} cm_packets={}",
                super::GC_COUNT.load(Ordering::SeqCst),
                pause,
                self.get_reserved_pages(),
                // Concurrent tracing packets still outstanding. If this is 0 at the end of an
                // InitialMark pause, no marking was handed to the concurrent workers and the
                // whole closure will fall into the next FinalMark pause.
                super::NUM_CONCURRENT_TRACING_PACKETS.load(Ordering::SeqCst),
            );
            eprintln!(
                "[lxr] gc#{} incs: processed={} promoted={} young_mb={} survival_ratio={:.4} predicted_survival_mb={} (limit {})",
                super::GC_COUNT.load(Ordering::SeqCst),
                super::INCS_PROCESSED.swap(0, Ordering::Relaxed),
                super::OBJS_PROMOTED.swap(0, Ordering::Relaxed),
                self.block_allocation.total_young_allocation_in_bytes() >> LOG_BYTES_IN_MBYTE,
                super::SURVIVAL_RATIO_PREDICTOR.promotion_ratio(),
                (((self.block_allocation.total_young_allocation_in_bytes()
                    >> LOG_BYTES_IN_MBYTE) as f64
                    * super::SURVIVAL_RATIO_PREDICTOR.promotion_ratio()) as usize)
                    << LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER,
                super::max_survival_mb(),
            );
            eprintln!(
                "[lxr] gc#{} slotless barrier: calls={} fields={} inc_buffer={}",
                super::GC_COUNT.load(Ordering::SeqCst),
                super::OBJ_WRITE_CALLS.swap(0, Ordering::Relaxed),
                super::OBJ_WRITE_FIELDS.swap(0, Ordering::Relaxed),
                self.rc.inc_buffer_size(),
            );
            eprintln!(
                "[lxr] gc#{} sweep dead cycles: zeroed={} kept_marked={}",
                super::GC_COUNT.load(Ordering::SeqCst),
                super::SWEEP_ZEROED.load(Ordering::Relaxed),
                super::SWEEP_KEPT_MARKED.load(Ordering::Relaxed),
            );
        }
        crate::scheduler::stage_timeline::mark("  release: weak+common");
        self.block_allocation
            .sweep_nursery_blocks(self.immix_space.scheduler(), pause);
        crate::scheduler::stage_timeline::mark("  release: sweep_nursery_blocks");
        self.block_allocation.sweep_mutator_reused_blocks(pause);
        crate::scheduler::stage_timeline::mark("  release: sweep_mutator_reused_blocks");
        // Check if we want to do all decs and sweeping in the pause
        if super::disable_lasy_dec_for_current_gc() {
            self.immix_space
                .scheduler()
                .process_concurrent_packets_in_pause();
        } else {
            debug_assert_ne!(pause, Pause::Full);
        }
        crate::scheduler::stage_timeline::mark("  release: concurrent_packets_in_pause");
        self.immix_space.release_rc();
        crate::scheduler::stage_timeline::mark("  release: release_rc");
        self.schedule_mature_sweeping(pause);
        crate::scheduler::stage_timeline::mark("  release: schedule_mature_sweeping");
        // Re-arm every object's log bit for the epoch that starts when mutators resume.
        //
        // The per-object log bit says "this object has not been snapshotted this epoch".
        // A binding whose inlined barrier fast path cannot name the field gates on it
        // (Julia does, in `mmtk_gc_wb_fast`), and the barrier clears it on the first write
        // so later writes in the same epoch are free. Nothing used to set it again:
        // promotion armed it once and that was all, so from the second epoch onwards the
        // barrier never fired for an object again. Every store after that went unrecorded,
        // which loses the increment for the newly stored value while the field's earlier
        // value still gets its decrement -- reclaiming objects that are still reachable.
        //
        // The per-field bits do not need this; `ProcessIncs::unlog_and_load_rc_object`
        // re-arms each field lazily as its increment is processed.
        //
        // Done here, at the end of the pause, because the bits must be set before mutators
        // resume and nothing in the pause itself consults them.
        // The whole-heap re-arm that used to happen here is gone. `ImmixSpace::set_side_log_bits`
        // walks every chunk in the heap single-threaded (and mmtk-core's own `warn!` there says
        // so), and `CommonPlan::set_side_log_bits` enumerates every LOS object one atomic store at
        // a time. That made the cost of a pause O(heap) rather than O(work done in the pause): it
        // measured 1.3ms of the 4.4ms median `RefCount` pause on `tree_mutable`, serially, in a
        // single `Release` packet, and it grows with the heap.
        //
        // It is also unnecessary. Only bits the barrier actually cleared need re-arming, and the
        // barrier now records what it cleared and re-arms exactly that on flush, in
        // `LXRFieldBarrierSemantics::rearm_logged_objects`. Mutators are flushed during every
        // pause, so the re-arm happens before mutators resume, which is the property this
        // location was chosen for.
        // swap roots
        let mut prev_roots = self.prev_roots.write().unwrap();
        let mut curr_roots = self.curr_roots.write().unwrap();
        std::mem::swap::<SegQueue<_>>(&mut prev_roots, &mut curr_roots);
        debug_assert!(curr_roots.is_empty());
    }

    fn get_collection_reserved_pages(&self) -> usize {
        let survival = {
            let predicted_survival = (self.block_allocation.clean_nursery_mb() as f64
                * super::SURVIVAL_RATIO_PREDICTOR.ratio())
                as usize;
            predicted_survival << LOG_CONSERVATIVE_SURVIVAL_RATIO_MULTIPLER
        };
        survival + self.immix_space.defrag_headroom_pages()
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

    /// Get a mutable reference to the common plan. See [`Self::common`].
    fn common_mut(&mut self) -> &mut CommonPlan<Self::VM> {
        &mut self.common
    }

    fn on_pause_start(&self, mmtk: &'static MMTK<Self::VM>) {
        super::NO_EVAC.store(false, Ordering::SeqCst);
        let pause = self.current_pause().unwrap();

        // Individual RC pauses that don't overlap with concurrent tracing consist of a GC cycle.
        // Concurrent tracing, including RC pauses in between, counts as one GC cycle.
        // A Full GC counts as a GC cycle.
        if pause == Pause::RefCount && !self.concurrent_work_in_progress()
            || pause == Pause::InitialMark
            || pause == Pause::Full
        {
            mmtk.gc_trigger.policy.on_gc_start(mmtk);
        }

        super::SURVIVAL_RATIO_PREDICTOR
            .set_alloc_size(self.block_allocation.total_young_allocation_in_bytes());

        if pause == Pause::Full || pause == Pause::InitialMark {
            // Reset block mark and object mark table.
            let work_packets = self.generate_full_trace_prepare_tasks();
            self.immix_space.scheduler().work_buckets[WorkBucketStage::RCProcessIncs]
                .bulk_add(work_packets);
        }

        for mutator in <VM as VMBinding>::VMActivePlan::mutators() {
            mutator.flush();
        }

        if pause == Pause::FinalMark {
            self.set_concurrent_marking_state(false);
        }
    }

    fn on_pause_end(&mut self, mmtk: &'static MMTK<Self::VM>, tls: VMWorkerThread) {
        super::DISABLE_LASY_DEC_FOR_CURRENT_GC.store(false, Ordering::SeqCst);
        // self.immix_space.flush_page_resource();
        let pause = self.current_pause().unwrap();
        if pause == Pause::InitialMark {
            self.set_concurrent_marking_state(true);
        }
        self.previous_pause.store(Some(pause), Ordering::SeqCst);
        self.current_pause.store(None, Ordering::SeqCst);
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
        // Snapshot the lazy-reclaim totals in the same breath as `HEAP_AFTER_GC`. Everything the
        // concurrent phase frees from here on is what this cycle reclaimed, so the two always
        // describe the same instant. They used to be reconstructed from a counter that
        // `LXR::prepare` zeroed at the start of every pause, which meant a decision taken just
        // after a pause read this cycle's reserved size against a counter that had just been
        // cleared -- "the heap is full and the collection freed nothing" -- and hinted an emergency
        // full trace. That accounted for every emergency pause on `tree_mutable`.
        self.lazy_freed_blocks_at_pause_end.store(
            self.num_clean_blocks_released_lazy.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        self.lazy_freed_los_pages_at_pause_end.store(
            self.los().num_pages_released_lazy.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        // Close this cycle's wave of deferred jobs and attribute it to this cycle, so that draining
        // it -- and only that -- reports the cycle's reclamation as complete.
        let epoch = super::GC_COUNT.load(Ordering::SeqCst);
        let outstanding = LAZY_SWEEPING_JOBS.write().swap(epoch);
        if outstanding == 0 {
            // Nothing was deferred, so no wave will report completion. Everything this cycle freed
            // is already visible, so decide now; otherwise `select_collection_kind` would block on
            // `wait_for_decide_cycle_collection` with nobody left to wake it.
            self.on_lazy_sweeping_finished(epoch);
        }

        self.common_mut().on_pause_end(tls);

        // Individual RC pauses that don't overlap with concurrent tracing consist of a GC cycle.
        // Concurrent tracing, including RC pauses in between, counts as one GC cycle.
        // A Full GC counts as a GC cycle.
        if pause == Pause::RefCount && !self.concurrent_work_in_progress()
            || pause == Pause::FinalMark
            || pause == Pause::Full
        {
            mmtk.gc_trigger.policy.on_gc_end(mmtk);
        }
    }

    fn root_scanning_stage(&self) -> WorkBucketStage {
        WorkBucketStage::RCProcessIncs
    }

    fn concurrent(&self) -> Option<&dyn ConcurrentPlan<VM = VM>> {
        Some(self)
    }
}

impl<VM: VMBinding> ConcurrentPlan for LXR<VM> {
    fn current_pause(&self) -> Option<Pause> {
        self.current_pause.load(Ordering::SeqCst)
    }

    fn concurrent_work_in_progress(&self) -> bool {
        self.in_concurrent_marking.load(Ordering::Acquire)
    }
}

impl<VM: VMBinding> LXR<VM> {
    pub fn new(args: CreateGeneralPlanArgs<VM>) -> Box<Self> {
        // Only evacuation forwards objects, so a binding that never evacuates (e.g. a
        // non-moving build) is free to keep its forwarding bits on the side.
        assert!(
            VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC.is_in_header()
                || !(super::NURSERY_EVACUATION || super::MATURE_EVACUATION),
            "LXR does not support placing forwarding bits on the side."
        );
        let num_workers = args.scheduler.num_workers();
        if std::env::var_os("MMTK_LXR_STATS").is_some() {
            // The cargo feature chain reaching these is long enough that it is worth
            // having the plan state what it actually compiled to, rather than deriving it
            // from the feature graph each time a question about evacuation comes up.
            eprintln!(
                "[lxr] config: nursery_evac={} mature_evac={} lazy_decs={} cm={} workers={}",
                super::NURSERY_EVACUATION,
                super::MATURE_EVACUATION,
                super::LAZY_DECREMENTS,
                !cfg!(feature = "lxr_no_cm"),
                num_workers,
            );
        }
        // Note: `Block::DEFRAG_STATE_TABLE` doesn't need to be listed here; it's already
        // registered unconditionally by `SideMetadataContext::new_global_specs` since every
        // Immix-family plan (not just LXR) requires it.
        let immix_specs = metadata::extract_side_metadata(&[
            MetadataSpec::OnSide(RC_TABLE),
            MetadataSpec::OnSide(
                *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
                    .as_spec()
                    .extract_side_spec(),
            ),
            // The per-object log bit has to be registered too, not just the per-field
            // one. LXR's own barrier only consults the field bits, but the plan declares
            // `needs_log_bit`, and bindings set the object bit directly (Julia does it
            // from its immortal post-alloc fast path). Leaving it out means its side
            // metadata is never mapped and the first such write faults.
            *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.as_spec(),
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
            avail_pages_at_end_of_last_gc: AtomicUsize::new(0),
            zeroing_packets_scheduled: AtomicBool::new(false),
            decide_cycle_collection: (Mutex::new(true), Condvar::new()),
            in_concurrent_marking: AtomicBool::new(false),
            prev_roots: Default::default(),
            curr_roots: Default::default(),
            rc: RefCountHelper::NEW,
            block_allocation: BlockAllocation::new(),
            evac_set: MatureEvacuationSet::default(),
            mature_evac_remset: MatureEvecRemSet::new(num_workers),
            possibly_dead_mature_blocks: Default::default(),
            num_clean_blocks_released_lazy: Default::default(),
            lazy_freed_blocks_at_pause_end: Default::default(),
            lazy_freed_los_pages_at_pause_end: Default::default(),
            decided_epoch: AtomicUsize::new(usize::MAX),
        });

        lxr.gc_init();

        // Note: `verify_side_metadata_sanity` is invoked later by `MMTK::new`, after the dynamic
        // side metadata base address has been initialized. It must not be called here during plan
        // construction, as the side metadata layout is not yet registered at this point.

        lxr
    }

    pub fn cm_enabled(&self) -> bool {
        !cfg!(feature = "lxr_no_cm")
    }

    fn schedule_defrag_selection_packets(&self) {
        self.evac_set
            .schedule_defrag_selection_packets(&self.immix_space)
    }

    /// Generate chunk sweep work packets.
    fn generate_dead_cycle_sweep_tasks(&self) -> Vec<Box<dyn GCWork<VM>>> {
        self.immix_space.chunk_map.generate_tasks_batched(
            self.immix_space.scheduler().num_workers(),
            |chunks| {
                Box::new(SweepDeadCycles::new(
                    chunks,
                    LazySweepingJobsCounter::new_decs(),
                ))
            },
        )
    }

    fn schedule_mature_sweeping(&self, pause: Pause) {
        if pause == Pause::Full || pause == Pause::FinalMark {
            self.evac_set
                .sweep_mature_evac_candidates(&self.immix_space);
            let disable_lasy_dec_for_current_gc =
                crate::plan::lxr::disable_lasy_dec_for_current_gc();
            let dead_cycle_sweep_packets = self.generate_dead_cycle_sweep_tasks();
            let sweep_los = RCSweepMatureAfterSATBLOS::new(LazySweepingJobsCounter::new_decs());
            if super::LAZY_DECREMENTS && !disable_lasy_dec_for_current_gc {
                debug_assert_ne!(pause, Pause::Full);
                let concurrent_bucket =
                    &self.immix_space.scheduler().work_buckets[WorkBucketStage::Concurrent];
                concurrent_bucket.bulk_add_deferred(dead_cycle_sweep_packets);
                concurrent_bucket.add_deferred(Box::new(sweep_los));
            } else {
                self.immix_space.scheduler().work_buckets[WorkBucketStage::STWRCDecsAndSweep]
                    .bulk_add(dead_cycle_sweep_packets);
                self.immix_space.scheduler().work_buckets[WorkBucketStage::STWRCDecsAndSweep]
                    .add(sweep_los);
            }
        }
    }

    /// Generate chunk sweep work packets.
    fn generate_full_trace_prepare_tasks(&self) -> Vec<Box<dyn GCWork<VM>>> {
        self.immix_space
            .chunk_map
            .generate_tasks_batched(self.immix_space.scheduler().num_workers(), |chunks| {
                Box::new(PrepareChunksForFullGC { chunks })
            })
    }

    fn schedule_rc_block_sweeping_tasks(&self, counter: LazySweepingJobsCounter) {
        // while let Some(x) = self.last_mutator_recycled_blocks.pop() {
        //     x.set_state(BlockState::Marked);
        // }
        // This may happen either within a pause, or in concurrent.
        let size = self.possibly_dead_mature_blocks.len();
        let num_bins = self.immix_space.scheduler().num_workers();
        let bin_cap = size / num_bins + if size % num_bins == 0 { 0 } else { 1 };
        let mut bins = (0..num_bins)
            .map(|_| Vec::with_capacity(bin_cap))
            .collect::<Vec<Vec<(Block, bool)>>>();
        'out: for bin in bins.iter_mut() {
            for _ in 0..bin_cap {
                if let Some(block) = self.possibly_dead_mature_blocks.pop() {
                    bin.push(block);
                } else {
                    break 'out;
                }
            }
        }
        let packets = bins
            .into_iter()
            .map::<Box<dyn GCWork<VM>>, _>(|blocks| {
                Box::new(SweepBlocksAfterDecs::new(blocks, counter.clone()))
            })
            .collect();
        self.immix_space.scheduler().work_buckets[WorkBucketStage::Unconstrained].bulk_add(packets);
    }

    pub(super) fn process_mature_evacuation_remset(&self) {
        self.mature_evac_remset.flush_all();
        let packets = self.mature_evac_remset.take_global_packets();
        self.immix_space.scheduler().work_buckets[WorkBucketStage::RCEvacuateMature]
            .bulk_add(packets);
    }

    pub(super) fn add_to_possibly_dead_mature_blocks(&self, block: Block, is_defrag_source: bool) {
        if block.log() {
            self.possibly_dead_mature_blocks
                .push((block, is_defrag_source));
        }
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

    fn next_gc_is_cycle_gc(&self, mature_space_pages: usize, pause: Pause) -> bool {
        if pause == Pause::FinalMark || pause == Pause::Full {
            super::MATURE_LIVE_PREDICTOR.update(mature_space_pages);
        }
        let live_mature_pages = super::MATURE_LIVE_PREDICTOR.live_pages() as usize;
        let garbage = mature_space_pages.saturating_sub(live_mature_pages);
        let total_pages = self.get_total_pages();
        !self.concurrent_work_in_progress()
            && (self.cm_enabled() && garbage * 100 >= super::TRACE_THRESHOLD * total_pages)
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
        let emergency_threshold = super::RC_STOP_PERCENT;
        // Calculate mature space size
        let total_pages = self.get_total_pages();
        // Reserved pages as of the end of the last pause, less everything the concurrent phase has
        // freed since. Both terms are deltas from the same instant -- see the snapshot in
        // `gc_pause_end`.
        let mature_space_pages = {
            let freed_blocks = self
                .num_clean_blocks_released_lazy
                .load(Ordering::SeqCst)
                .saturating_sub(self.lazy_freed_blocks_at_pause_end.load(Ordering::SeqCst));
            let freed_los_pages = self
                .los()
                .num_pages_released_lazy
                .load(Ordering::SeqCst)
                .saturating_sub(
                    self.lazy_freed_los_pages_at_pause_end
                        .load(Ordering::SeqCst),
                );
            HEAP_AFTER_GC
                .load(Ordering::SeqCst)
                .saturating_sub(freed_blocks << Block::LOG_PAGES)
                .saturating_sub(freed_los_pages)
        };
        // Decide next GC kind
        let hint_cycle_gc = self.next_gc_is_cycle_gc(mature_space_pages, pause);
        let hint_emergency_gc =
            self.next_gc_is_emergency_gc(total_pages, mature_space_pages, emergency_threshold);
        if super::stats() {
            eprintln!(
                "[lxr] decide: total_pages={} mature_pages={} heap_after_gc={} lazy_freed_blocks={} hint_cycle={} hint_emergency={}",
                total_pages,
                mature_space_pages,
                HEAP_AFTER_GC.load(Ordering::SeqCst),
                self.num_clean_blocks_released_lazy.load(Ordering::SeqCst),
                hint_cycle_gc,
                hint_emergency_gc,
            );
        }
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
        let work_packets = self
            .immix_space
            .chunk_map
            .generate_tasks_batched(self.immix_space.scheduler().num_workers(), |chunks| {
                Box::new(ConcurrentChunkMetadataZeroing { chunks })
            });
        self.immix_space.scheduler().work_buckets[WorkBucketStage::Unconstrained]
            .bulk_add(work_packets);
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

        // Bring-up aid: never trace. With concurrent marking off, `Full` is the only pause
        // that marks and sweeps mature blocks, so forcing `RefCount` isolates the
        // reference-counting path from the tracing path. Cycles are then never collected,
        // so the heap only grows -- diagnostic only. Set `MMTK_LXR_NO_FULL=1`.
        if super::no_full_pauses() {
            return Pause::RefCount;
        }

        let emergency = self.base().global_state.is_emergency_collection();
        let user_triggered = self.base().global_state.is_user_triggered_collection();
        let cm_in_progress = self.concurrent_work_in_progress();
        let cm_packets_drained = super::concurrent_marking_packets_drained();
        let hint_cycle_gc = self.hint_cycle_gc.load(Ordering::SeqCst);
        let hint_emergency_gc = self.hint_emergency_gc.load(Ordering::SeqCst);
        // If CM is finished, do a final mark pause
        if cm_in_progress && cm_packets_drained {
            return Pause::FinalMark;
        }

        if crate::plan::lxr::stats() && (emergency || user_triggered || hint_emergency_gc) {
            eprintln!(
                "[lxr] emergency: emergency={} user_triggered={} hint_emergency={} cm_in_progress={}",
                emergency, user_triggered, hint_emergency_gc, cm_in_progress
            );
        }

        // A real emergency: mmtk-core could not satisfy an allocation even after a collection, or
        // the user asked for a full collection. Stopping the world for the whole trace is the
        // correct response -- there is no budget left to trace concurrently in.
        if emergency || user_triggered {
            return if cm_in_progress {
                Pause::FinalMark
            } else {
                Pause::Full
            };
        }

        // Free headroom is below `RC_STOP_PERCENT`, so reference counting alone is not keeping up
        // and the heap needs a trace to find its cycles. That is a reason to *trace*, not a reason
        // to stop the world: tracing concurrently is what this plan exists for. An `InitialMark`
        // pause plus the `FinalMark` that closes it measured ~2.4ms + ~17ms on `tree_mutable`
        // against ~300ms for the full stop-the-world trace this used to choose, and those full
        // traces were 70% of all pause time.
        //
        // The hint fires readily because the headroom test compares the heap budget against
        // `HEAP_AFTER_GC` less the whole blocks deferred sweeping returned, and LXR reclaims mostly
        // by recycling lines inside blocks that stay reserved. Reserved pages therefore sit near the
        // budget whether or not memory is actually short, so this hint cannot carry the weight of a
        // whole-heap stop -- and does not need to, since a genuine exhaustion still arrives as
        // `emergency` above.
        if hint_emergency_gc {
            return if cm_in_progress {
                Pause::FinalMark
            } else if self.cm_enabled() {
                Pause::InitialMark
            } else {
                Pause::Full
            };
        }

        // Should trigger CM?
        if hint_cycle_gc && !cm_in_progress {
            if self.cm_enabled() {
                Pause::InitialMark
            } else {
                Pause::Full
            }
        } else {
            Pause::RefCount
        }
    }

    fn disable_unnecessary_buckets(&'static self, scheduler: &GCWorkScheduler<VM>, pause: Pause) {
        // Set conditional buckets
        scheduler.work_buckets[WorkBucketStage::RCProcessIncs].set_enabled(true);
        scheduler.work_buckets[WorkBucketStage::Prepare].set_enabled(pause != Pause::RefCount);
        let final_mark_or_full = pause == Pause::FinalMark || pause == Pause::Full;
        scheduler.work_buckets[WorkBucketStage::Closure].set_enabled(final_mark_or_full);
        scheduler.work_buckets[WorkBucketStage::WeakRefClosure].set_enabled(final_mark_or_full);
        scheduler.work_buckets[WorkBucketStage::FinalRefClosure].set_enabled(final_mark_or_full);
        scheduler.work_buckets[WorkBucketStage::PhantomRefClosure].set_enabled(final_mark_or_full);
        // The VM's own reference processing, which for Julia is where finalizers are swept.
        //
        // This was disabled outright, and with the bucket disabled MMTk never calls
        // `Scanning::process_weak_refs` at all. Two things then never ran. Finalizer lists
        // were never swept, so nothing was ever finalized while the program ran; and
        // `mark_finlist`, which traces the objects still on those lists, never ran either --
        // and that trace is what keeps a registered object alive until its finalizer has been
        // scheduled. Registered objects were therefore reclaimed with live entries still
        // naming them, and `jl_gc_run_all_finalizers` at exit ran every one of those entries
        // against recycled memory: an invalid `free` on the C finalizer path
        // (`gc-common.c:174`), a segfault reading the argument's type on the Julia path
        // (`gc-common.c:180`).
        //
        // The same call also schedules `SweepVMSpecific`, so `jl_gc_sweep_weak_processing`,
        // `jl_gc_mmtk_sweep_malloced_memory` and `jl_gc_sweep_stack_pools_and_mtarraylist_buffers`
        // were all being skipped as well.
        //
        // Still disabled, because enabling it is necessary but not sufficient and the rest is
        // not yet working. Two further things are needed, both attempted and reverted:
        //
        //  1. A packet in the bucket. Plans that use `GCWorkScheduler::schedule_common_work`
        //     get a `VMProcessWeakRefs` sentinel from there; LXR schedules its own collection,
        //     and that sentinel is commented out on the upstream LXR branch regardless.
        //     ConcurrentImmix sets it explicitly with `PlanTrace<_, TRACE_KIND_FAST>`; LXR is
        //     not a `PlanTraceObject`, so `gc_work::tracing::LXRRefTrace` exists for this.
        //  2. A liveness predicate that is safe here. `sweep_finalizer_list` asks
        //     `is_live_object`, and `ImmixSpace::is_live`'s `rc_enabled` branch sends every
        //     *unmarked* object -- i.e. every dead entry, which is what the sweep is looking
        //     for -- into `object_forwarding::is_forwarded`. Under `lxr_no_evac` nothing moves,
        //     that state is never established, and reading it faults. `object_is_live` in
        //     `mmtk_julia/src/julia_finalizer.rs` answers in LXR's own terms instead.
        //
        // With both in place the first tracing pause still segfaults, so this stays off.
        scheduler.work_buckets[WorkBucketStage::VMRefClosure].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep]
            .set_enabled(!(super::LAZY_DECREMENTS && pause != Pause::Full));
        // Always enabled
        scheduler.work_buckets[WorkBucketStage::Concurrent].set_enabled(true);
        scheduler.work_buckets[WorkBucketStage::ConcurrentResumable].set_enabled(true);
        // Always disabled
        scheduler.work_buckets[WorkBucketStage::TPinningClosure].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::PinningRootsTrace].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::VMRefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::SoftRefClosure].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::CalculateForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::SecondRoots].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::RefForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::FinalizableForwarding].set_enabled(false);
        scheduler.work_buckets[WorkBucketStage::Compact].set_enabled(false);
    }

    fn schedule_rc_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        log::info!("Scheduling RC collection...");
        self.disable_unnecessary_buckets(scheduler, Pause::RefCount);
        // Before start yielding, wrap all the roots from the previous GC with work-packets.
        self.process_prev_roots(scheduler);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<LXRGCWorkContext<VM>>::new_with_flush());
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::RCProcessIncs].add(FastRCPrepare);
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<VM>>::new(self));
    }

    fn schedule_concurrent_marking_initial_pause(&'static self, scheduler: &GCWorkScheduler<VM>) {
        log::info!("Scheduling concurrent marking initial pause...");
        self.disable_unnecessary_buckets(scheduler, Pause::InitialMark);
        self.process_prev_roots(scheduler);
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<LXRGCWorkContext<VM>>::new_with_flush());
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<LXRGCWorkContext<VM>>::new(self));
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<VM>>::new(self));
    }

    fn schedule_concurrent_marking_final_pause(&'static self, scheduler: &GCWorkScheduler<VM>) {
        log::info!("Scheduling concurrent marking final pause...");
        self.disable_unnecessary_buckets(scheduler, Pause::FinalMark);
        self.process_prev_roots(scheduler);
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<LXRGCWorkContext<VM>>::new_with_flush());

        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<LXRGCWorkContext<VM>>::new(self));
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<VM>>::new(self));
    }

    fn schedule_emergency_full_heap_collection(&'static self, scheduler: &GCWorkScheduler<VM>) {
        log::info!("Scheduling emergency full-heap collection...");
        super::DISABLE_LASY_DEC_FOR_CURRENT_GC.store(true, Ordering::SeqCst);
        self.disable_unnecessary_buckets(scheduler, Pause::Full);
        // Before start yielding, wrap all the roots from the previous GC with work-packets.
        self.process_prev_roots(scheduler);
        // Stop & scan mutators (mutator scanning can happen before STW)
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<LXRGCWorkContext<VM>>::new_with_flush());
        // Prepare global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(Prepare::<LXRGCWorkContext<VM>>::new(self));
        // Release global/collectors/mutators
        scheduler.work_buckets[WorkBucketStage::Release]
            .add(Release::<LXRGCWorkContext<VM>>::new(self));
    }

    fn process_prev_roots(&self, scheduler: &GCWorkScheduler<VM>) {
        let prev_roots = self.prev_roots.read().unwrap();
        let mut work_packets: Vec<Box<dyn GCWork<VM>>> = Vec::with_capacity(prev_roots.len());
        while let Some(decs) = prev_roots.pop() {
            let mut w = ProcessDecs::new(decs, LazySweepingJobsCounter::new_decs());
            w.origin = "prev_roots";
            work_packets.push(Box::new(w))
        }
        if work_packets.is_empty() {
            work_packets.push(Box::new(ProcessDecs::new(
                vec![],
                LazySweepingJobsCounter::new_decs(),
            )));
        }
        if super::LAZY_DECREMENTS {
            scheduler.work_buckets[WorkBucketStage::Concurrent].bulk_add_deferred(work_packets);
        } else {
            scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].bulk_add(work_packets);
        }
    }

    pub fn current_pause(&self) -> Option<Pause> {
        self.current_pause.load(Ordering::SeqCst)
    }

    pub fn previous_pause(&self) -> Option<Pause> {
        self.previous_pause.load(Ordering::SeqCst)
    }

    /// Returns whether the given object is in a block that was selected for
    /// defragmentation (evacuation) in the current collection. Only Immix space
    /// objects live in blocks, so this is false for everything else.
    pub fn in_defrag(&self, o: ObjectReference) -> bool {
        self.immix_space.in_space(o) && Block::in_defrag_block(o)
    }

    pub fn address_in_defrag(&self, a: Address) -> bool {
        self.immix_space.address_in_space(a) && Block::address_in_defrag_block(a)
    }

    /// Attempts to mark the object as live, in whichever space (Immix or large
    /// object space) it belongs to. Returns `true` if this call performed the
    /// marking (i.e. the object was previously unmarked).
    ///
    /// Objects owned by the common plan have no mark state that LXR maintains, and
    /// are unconditionally live, so marking them is a no-op that reports no work
    /// done.
    pub fn mark(&self, o: ObjectReference) -> bool {
        match self.space_of(o) {
            LXRSpace::Immix => self.immix_space.attempt_mark(o),
            LXRSpace::Los => self.common.los.attempt_mark(o),
            LXRSpace::Common => false,
        }
    }

    /// Like [`Self::mark`], but takes an explicit `los` flag indicating whether
    /// the object is in the large object space, avoiding a space lookup.
    pub fn mark2(&self, o: ObjectReference, los: bool) -> bool {
        if !los {
            self.immix_space.attempt_mark(o)
        } else {
            self.common.los.attempt_mark(o)
        }
    }

    /// Returns whether the object has already been marked as live in whichever
    /// space (Immix or large object space) it belongs to.
    ///
    /// Objects owned by the common plan are never reclaimed, so they are always
    /// reported as marked. Callers use this to decide whether an object still needs
    /// to be retained or revisited, and neither is ever true for them.
    pub fn is_marked(&self, o: ObjectReference) -> bool {
        match self.space_of(o) {
            LXRSpace::Immix => self.immix_space.is_marked(o),
            LXRSpace::Los => self.common.los.is_marked(o),
            LXRSpace::Common => true,
        }
    }

    pub const fn los(&self) -> &LargeObjectSpace<VM> {
        &self.common.los
    }

    fn on_lazy_decs_finished(&self, c: LazySweepingJobsCounter) {
        self.schedule_rc_block_sweeping_tasks(c);
    }

    fn on_lazy_sweeping_finished(&self, epoch: super::WaveEpoch) {
        // Always worth doing: it makes whatever was just freed visible, whichever wave this was.
        self.immix_space.flush_page_resource();
        // Update counters
        if !super::LAZY_DECREMENTS {
            HEAP_AFTER_GC.store(self.get_used_pages(), Ordering::SeqCst);
        }
        // Only the wave belonging to the most recent pause means "that cycle's reclamation is
        // done". A still-open wave draining transiently, or a wave for an older cycle arriving
        // late, would otherwise pair the latest `HEAP_AFTER_GC` with reclamation that has not
        // happened yet -- which is what hinted an emergency and forced a full trace.
        if epoch != super::GC_COUNT.load(Ordering::SeqCst) {
            return;
        }
        // And only once per cycle: two waves can both belong to it.
        if self.decided_epoch.swap(epoch, Ordering::SeqCst) == epoch {
            return;
        }
        let pause = match self.current_pause() {
            Some(p) => p,
            None => self.previous_pause().unwrap(),
        };
        self.decide_next_gc_may_perform_cycle_collection(pause);
        // The flush above is the first moment this cycle's reclamation is visible through
        // `get_reserved_pages`, so it is the first moment a trigger policy can size the next
        // heap target from what the collection actually freed.
        self.base().gc_trigger.policy.on_lazy_reclaim_finished(self);
    }

    fn gc_init(&mut self) {
        self.immix_space.rc_enabled = true;
        self.common.los.rc_enabled = true;
        unsafe {
            let me: &'static Self = &*(self as *const Self);
            me.block_allocation.init(&me.immix_space, me);
            me.immix_space.install_hooks(&me.block_allocation);
        }
        let mut lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.write();
        // Prime the counters. There is no cycle yet, so the wave this closes belongs to none.
        lazy_sweeping_jobs.swap(super::WAVE_STILL_OPEN);
        let lxr_ptr = self as *const Self as usize;
        lazy_sweeping_jobs.end_of_decs = Some(Box::new(move |c| {
            let lxr = unsafe { &*(lxr_ptr as *const Self) };
            lxr.on_lazy_decs_finished(c);
        }));
        lazy_sweeping_jobs.end_of_lazy = Some(Box::new(move |epoch| {
            let lxr = unsafe { &*(lxr_ptr as *const Self) };
            lxr.on_lazy_sweeping_finished(epoch);
        }));
    }

    fn set_concurrent_marking_state(&self, active: bool) {
        self.in_concurrent_marking.store(active, Ordering::SeqCst);
        self.common
            .los
            .bump_page_reuse_count
            .store(active, Ordering::SeqCst);
    }
}
