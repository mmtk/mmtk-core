use super::block_allocation::BlockAllocation;
use super::defrag::StatsForDefrag;
use super::line::*;
use super::rc_work::*;
use super::{block::*, defrag::Defrag};
use crate::plan::immix::Pause;
use crate::plan::lxr::MatureEvecRemSet;
use crate::plan::VectorObjectQueue;
use crate::policy::gc_work::{TraceKind, DEFAULT_TRACE, TRACE_KIND_TRANSITIVE_PIN};
use crate::policy::largeobjectspace::RCSweepMatureAfterSATBLOS;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::sft_map::SFTMap;
use crate::policy::space::{CommonSpace, Space};
use crate::util::alloc::allocator::AllocationOptions;
use crate::util::alloc::allocator::AllocatorContext;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::heap::chunk_map::*;
use crate::util::heap::BlockPageResource;
use crate::util::heap::PageResource;
use crate::util::linear_scan::Region;
use crate::util::metadata::log_bit::UnlogBitsOperation;
use crate::util::metadata::side_metadata::spec_defs::IX_LINE_REUSE_COUNT;
use crate::util::metadata::side_metadata::*;
#[cfg(feature = "vo_bit")]
use crate::util::metadata::vo_bit;
use crate::util::metadata::{self, MetadataSpec};
use crate::util::object_enum::ObjectEnumerator;
use crate::util::object_forwarding;
use crate::util::rc::RefCountHelper;
use crate::util::{copy::*, epilogue, object_enum};
use crate::util::{Address, ObjectReference};
use crate::{
    plan::ObjectQueue,
    scheduler::{GCWork, GCWorkScheduler, GCWorker, WorkBucketStage},
    util::opaque_pointer::{VMThread, VMWorkerThread},
    MMTK,
};
use crate::{vm::*, LazySweepingJobsCounter};
use atomic::Ordering;
use crossbeam::queue::SegQueue;
use std::mem;
use std::ops::Range;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::sync::{atomic::AtomicU8, Arc};

pub static RELEASED_NURSERY_BLOCKS: AtomicUsize = AtomicUsize::new(0);
pub static RELEASED_BLOCKS: AtomicUsize = AtomicUsize::new(0);

pub(crate) const TRACE_KIND_FAST: TraceKind = 0;
pub(crate) const TRACE_KIND_DEFRAG: TraceKind = 1;

pub struct ImmixSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pub pr: BlockPageResource<VM, Block>,
    /// Allocation status for all chunks in immix space
    pub chunk_map: ChunkMap,
    /// Current line mark state
    pub line_mark_state: AtomicU8,
    /// Line mark state in previous GC
    line_unavail_state: AtomicU8,
    /// A list of all reusable blocks
    pub reusable_blocks: ReusableBlockPool,
    /// Defrag utilities
    pub(super) defrag: Defrag,
    /// How many lines have been consumed since last GC?
    lines_consumed: AtomicUsize,
    reused_lines_consumed: AtomicUsize,
    /// Object mark state
    mark_state: u8,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// Some settings for this space
    space_args: ImmixSpaceArgs,
    pub block_allocation: BlockAllocation<VM>,
    possibly_dead_mature_blocks: SegQueue<(Block, bool)>,
    initial_mark_pause: bool,
    pub mature_evac_remsets: Mutex<Vec<Box<dyn GCWork<VM>>>>,
    pub num_clean_blocks_released_young: AtomicUsize,
    pub num_clean_blocks_released_mature: AtomicUsize,
    pub num_clean_blocks_released_lazy: AtomicUsize,
    pub copy_alloc_bytes: AtomicUsize,
    pub rc_killed_bytes: AtomicUsize,
    pub mature_evac_remset: MatureEvecRemSet<VM>,
    pub cm_enabled: bool,
    pub rc_enabled: bool,
    pub is_end_of_satb_or_full_gc: bool,
    pub rc: RefCountHelper<VM>,
    pub(super) evac_set: MatureEvacuationSet,
}

/// Some arguments for Immix Space.
pub struct ImmixSpaceArgs {
    /// Whether this ImmixSpace instance contains both young and old objects.
    /// This affects the updating of valid-object bits.  If some lines or blocks of this ImmixSpace
    /// instance contain young objects, their VO bits need to be updated during this GC.  Currently
    /// only StickyImmix is affected.  GenImmix allocates young objects in a separete CopySpace
    /// nursery and its VO bits can be cleared in bulk.
    pub mixed_age: bool,
    /// Disable copying for this Immix space.
    pub never_move_objects: bool,
}

unsafe impl<VM: VMBinding> Sync for ImmixSpace<VM> {}

impl<VM: VMBinding> SFT for ImmixSpace<VM> {
    fn name(&self) -> &'static str {
        self.get_name()
    }

    fn get_forwarded_object(&self, object: ObjectReference) -> Option<ObjectReference> {
        if object_forwarding::is_forwarded::<VM>(object) {
            Some(object_forwarding::read_forwarding_pointer::<VM>(object))
        } else {
            None
        }
    }

    fn is_live(&self, object: ObjectReference) -> bool {
        if self.rc_enabled {
            if self.is_end_of_satb_or_full_gc {
                if self.is_marked(object) {
                    let block = Block::containing(object);
                    if block.is_defrag_source() {
                        if object_forwarding::is_forwarded::<VM>(object) {
                            let forwarded =
                                object_forwarding::read_forwarding_pointer::<VM>(object);
                            return self.is_marked(forwarded) && self.rc.count(forwarded) > 0;
                        } else {
                            return false;
                        }
                    }
                    return self.rc.count(object) > 0;
                } else if object_forwarding::is_forwarded::<VM>(object) {
                    let forwarded = object_forwarding::read_forwarding_pointer::<VM>(object);
                    debug_assert!(
                        forwarded.to_raw_address().is_mapped(),
                        "Invalid forwarded object: {:?} -> {:?}",
                        object,
                        forwarded
                    );
                    return self.is_marked(forwarded) && self.rc.count(forwarded) > 0;
                } else {
                    return false;
                }
            }
            return self.rc.count(object) > 0 || object_forwarding::is_forwarded::<VM>(object);
        }
        if self.initial_mark_pause {
            return true;
        }
        if self.cm_enabled {
            let block_state = Block::containing(object).get_state();
            if block_state == BlockState::Nursery {
                return true;
            }
        }

        // If the mark bit is set, it is live.
        if self.is_marked(object) {
            return true;
        }

        // If we never move objects, look no further.
        if !self.is_movable() {
            return false;
        }

        // If the object is forwarded, it is live, too.
        object_forwarding::is_forwarded::<VM>(object)
    }

    fn is_reachable(&self, object: ObjectReference) -> bool {
        if self.rc_enabled {
            if object_forwarding::is_forwarded::<VM>(object) {
                let forwarded = object_forwarding::read_forwarding_pointer::<VM>(object);
                return self.is_marked(forwarded) && self.rc.count(forwarded) > 0;
            }
            return self.is_marked(object) && self.rc.count(object) > 0;
        } else {
            self.is_live(object)
        }
    }
    #[cfg(feature = "object_pinning")]
    fn pin_object(&self, object: ObjectReference) -> bool {
        VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC.pin_object::<VM>(object)
    }
    #[cfg(feature = "object_pinning")]
    fn unpin_object(&self, object: ObjectReference) -> bool {
        VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC.unpin_object::<VM>(object)
    }
    #[cfg(feature = "object_pinning")]
    fn is_object_pinned(&self, object: ObjectReference) -> bool {
        VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC.is_object_pinned::<VM>(object)
    }
    fn is_movable(&self) -> bool {
        !self.space_args.never_move_objects
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_object_metadata(&self, _object: ObjectReference, _bytes: usize) {
        self.copy_alloc_bytes.store(0, Ordering::SeqCst);
        #[cfg(feature = "vo_bit")]
        crate::util::metadata::vo_bit::set_vo_bit(_object);
    }
    #[cfg(feature = "is_mmtk_object")]
    fn is_mmtk_object(&self, addr: Address) -> Option<ObjectReference> {
        crate::util::metadata::vo_bit::is_vo_bit_set_for_addr(addr)
    }
    #[cfg(feature = "is_mmtk_object")]
    fn find_object_from_internal_pointer(
        &self,
        ptr: Address,
        max_search_bytes: usize,
    ) -> Option<ObjectReference> {
        // We don't need to search more than the max object size in the immix space.
        let search_bytes = usize::min(super::MAX_IMMIX_OBJECT_SIZE, max_search_bytes);
        crate::util::metadata::vo_bit::find_object_from_internal_pointer::<VM>(ptr, search_bytes)
    }
    fn sft_trace_object(
        &self,
        _queue: &mut VectorObjectQueue,
        _object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        panic!("We do not use SFT to trace objects for Immix. sft_trace_object() cannot be used.")
    }

    fn debug_print_object_info(&self, object: ObjectReference) {
        println!("marked  = {}", self.is_marked(object));
        println!(
            "line marked = {}",
            Line::from_unaligned_address(object.to_raw_address()).is_marked(self.mark_state)
        );
        println!(
            "block state = {:?}",
            Block::from_unaligned_address(object.to_raw_address()).get_state()
        );
        object_forwarding::debug_print_object_forwarding_info::<VM>(object);
        self.common.debug_print_object_global_info(object);
    }
}

impl<VM: VMBinding> Space<VM> for ImmixSpace<VM> {
    fn as_space(&self) -> &dyn Space<VM> {
        self
    }
    fn as_sft(&self) -> &(dyn SFT + Sync + 'static) {
        self
    }
    fn get_page_resource(&self) -> &dyn PageResource<VM> {
        &self.pr
    }
    fn maybe_get_page_resource_mut(&mut self) -> Option<&mut dyn PageResource<VM>> {
        Some(&mut self.pr)
    }
    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }
    fn initialize_sft(&self, sft_map: &mut dyn SFTMap) {
        self.common().initialize_sft(self.as_sft(), sft_map);
        // Initialize the block queues in `reusable_blocks` and `pr`.
        self.block_allocation.init(self);
    }
    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immixspace only releases pages enmasse")
    }
    fn set_copy_for_sft_trace(&mut self, _semantics: Option<CopySemantics>) {
        panic!("We do not use SFT to trace objects for Immix. set_copy_context() cannot be used.")
    }

    fn enumerate_objects(&self, enumerator: &mut dyn ObjectEnumerator) {
        object_enum::enumerate_blocks_from_chunk_map::<Block>(enumerator, &self.chunk_map);
    }

    fn clear_side_log_bits(&self) {
        // Remove the following warning if we have a legitimate use case.
        warn!("ImmixSpace::clear_side_log_bits is single-treaded.  Consider clearing side metadata in per-chunk work packets.");

        let log_bit = VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.extract_side_spec();
        for chunk in self.chunk_map.all_chunks() {
            log_bit.bzero_metadata(chunk.start(), Chunk::BYTES);
        }
    }

    fn set_side_log_bits(&self) {
        // Remove the following warning if we have a legitimate use case.
        warn!("ImmixSpace::set_side_log_bits is single-treaded.  Consider setting side metadata in per-chunk work packets.");

        let log_bit = VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.extract_side_spec();
        for chunk in self.chunk_map.all_chunks() {
            log_bit.bset_metadata(chunk.start(), Chunk::BYTES);
        }
    }
}

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for ImmixSpace<VM> {
    fn trace_object<Q: ObjectQueue, const KIND: TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        copy: Option<CopySemantics>,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        if KIND == TRACE_KIND_TRANSITIVE_PIN {
            self.trace_object_without_moving(queue, object)
        } else if KIND == TRACE_KIND_DEFRAG {
            if Block::containing(object).is_defrag_source() {
                debug_assert!(self.in_defrag());
                debug_assert!(
                    !crate::plan::is_nursery_gc(worker.mmtk.get_plan()),
                    "Calling PolicyTraceObject on Immix in nursery GC"
                );
                self.trace_object_with_opportunistic_copy(
                    queue,
                    object,
                    copy.unwrap(),
                    worker,
                    // This should not be nursery collection. Nursery collection does not use PolicyTraceObject.
                    false,
                )
            } else {
                self.trace_object_without_moving(queue, object)
            }
        } else if KIND == TRACE_KIND_FAST {
            self.trace_object_without_moving(queue, object)
        } else {
            unreachable!()
        }
    }

    fn post_scan_object(&self, object: ObjectReference) {
        if super::MARK_LINE_AT_SCAN_TIME && !super::BLOCK_ONLY {
            debug_assert!(self.in_space(object));
            self.mark_lines(object);
        }
    }

    #[allow(clippy::if_same_then_else)] // DEFAULT_TRACE needs a workaround which is documented below.
    fn may_move_objects<const KIND: TraceKind>() -> bool {
        if KIND == TRACE_KIND_DEFRAG {
            true
        } else if KIND == TRACE_KIND_FAST || KIND == TRACE_KIND_TRANSITIVE_PIN {
            false
        } else if KIND == DEFAULT_TRACE {
            // FIXME: This is hacky. When we do a default trace, this should be a nonmoving space.
            // The only exception is the nursery GC for sticky immix, for which, we use default trace.
            // This function is only used for PlanProcessEdges, and for sticky immix nursery GC, we use
            // GenNurseryProcessEdges. So it still works. But this is quite hacky anyway.
            // See https://github.com/mmtk/mmtk-core/issues/1314 for details.
            false
        } else {
            unreachable!()
        }
    }
}

impl<VM: VMBinding> ImmixSpace<VM> {
    #[allow(unused)]
    const UNMARKED_STATE: u8 = 0;
    const MARKED_STATE: u8 = 1;

    /// Get side metadata specs
    fn side_metadata_specs(rc_enabled: bool) -> Vec<SideMetadataSpec> {
        if rc_enabled {
            let meta = vec![
                MetadataSpec::OnSide(Block::MARK_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                MetadataSpec::OnSide(crate::util::rc::RC_STRADDLE_LINES),
                MetadataSpec::OnSide(Block::LOG_TABLE),
                MetadataSpec::OnSide(Block::NURSERY_PROMOTION_STATE_TABLE),
                MetadataSpec::OnSide(Block::DEAD_WORDS),
                MetadataSpec::OnSide(IX_LINE_REUSE_COUNT),
            ];
            return metadata::extract_side_metadata(&meta);
        }
        metadata::extract_side_metadata(&if super::BLOCK_ONLY {
            vec![
                MetadataSpec::OnSide(Block::MARK_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                *VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
                *VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
                #[cfg(feature = "object_pinning")]
                *VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC,
            ]
        } else {
            vec![
                MetadataSpec::OnSide(Line::MARK_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                *VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC,
                *VM::VMObjectModel::LOCAL_FORWARDING_POINTER_SPEC,
                #[cfg(feature = "object_pinning")]
                *VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC,
            ]
        })
    }

    pub fn new(
        args: crate::policy::space::PlanCreateSpaceArgs<VM>,
        mut space_args: ImmixSpaceArgs,
    ) -> Self {
        if args.unlog_traced_object {
            assert!(
                args.constraints.needs_log_bit,
                "Invalid args when the plan does not use log bit"
            );
        }

        // Make sure we override the space args if we force non moving Immix
        if cfg!(feature = "immix_non_moving") && !space_args.never_move_objects {
            info!(
                "Overriding never_moves_objects for Immix Space {}, as the immix_non_moving feature is set. Block size: 2^{}",
                args.name,
                Block::LOG_BYTES,
            );
            space_args.never_move_objects = true;
        }

        // validate features
        if super::BLOCK_ONLY {
            assert!(
                space_args.never_move_objects,
                "Block-only immix must not move objects"
            );
        }
        assert!(
            Block::LINES / 2 <= u8::MAX as usize - 2,
            "Number of lines in a block should not exceed BlockState::MARK_MARKED"
        );

        #[cfg(feature = "vo_bit")]
        vo_bit::helper::validate_config::<VM>();
        let vm_map = args.vm_map;
        let scheduler = args.scheduler.clone();
        let rc_enabled = args.constraints.rc_enabled;
        let common = CommonSpace::new(args.into_policy_args(
            true,
            false,
            Self::side_metadata_specs(rc_enabled),
        ));
        let space_index = common.descriptor.get_index();
        ImmixSpace {
            pr: if common.vmrequest.is_discontiguous() {
                BlockPageResource::new_discontiguous(
                    Block::LOG_PAGES,
                    vm_map,
                    scheduler.num_workers(),
                )
            } else {
                BlockPageResource::new_contiguous(
                    Block::LOG_PAGES,
                    common.start,
                    common.extent,
                    vm_map,
                    scheduler.num_workers(),
                )
            },
            common,
            chunk_map: ChunkMap::new(space_index),
            line_mark_state: AtomicU8::new(Line::RESET_MARK_STATE),
            line_unavail_state: AtomicU8::new(Line::RESET_MARK_STATE),
            lines_consumed: AtomicUsize::new(0),
            reused_lines_consumed: AtomicUsize::new(0),
            reusable_blocks: ReusableBlockPool::new(scheduler.num_workers()),
            defrag: Defrag::default(),
            // Set to the correct mark state when inititialized. We cannot rely on prepare to set it (prepare may get skipped in nursery GCs).
            mark_state: Self::MARKED_STATE,
            mature_evac_remset: MatureEvecRemSet::new(scheduler.num_workers()),
            scheduler,
            space_args,
            block_allocation: BlockAllocation::new(),
            possibly_dead_mature_blocks: Default::default(),
            initial_mark_pause: false,
            mature_evac_remsets: Default::default(),
            num_clean_blocks_released_young: Default::default(),
            num_clean_blocks_released_mature: Default::default(),
            num_clean_blocks_released_lazy: Default::default(),
            copy_alloc_bytes: Default::default(),
            rc_killed_bytes: Default::default(),
            cm_enabled: false,
            rc_enabled,
            is_end_of_satb_or_full_gc: false,
            rc: RefCountHelper::NEW,
            evac_set: MatureEvacuationSet::default(),
        }
    }

    /// Flush the thread-local queues in BlockPageResource
    pub fn flush_page_resource(&self) {
        #[cfg(target_pointer_width = "64")]
        self.pr.flush_all()
    }

    /// Get the number of defrag headroom pages.
    pub fn defrag_headroom_pages(&self) -> usize {
        self.defrag.defrag_headroom_pages(self)
    }

    /// Check if current GC is a defrag GC.
    pub fn in_defrag(&self) -> bool {
        self.defrag.in_defrag()
    }

    /// check if the current GC should do defragmentation.
    pub fn decide_whether_to_defrag(
        &self,
        emergency_collection: bool,
        collect_whole_heap: bool,
        collection_attempts: usize,
        user_triggered_collection: bool,
        full_heap_system_gc: bool,
    ) -> bool {
        self.defrag.decide_whether_to_defrag(
            self.is_defrag_enabled(),
            emergency_collection,
            collect_whole_heap,
            collection_attempts,
            user_triggered_collection,
            self.reusable_blocks.len() == 0,
            full_heap_system_gc,
            self.cm_enabled,
            self.rc_enabled,
            *self.common.options.immix_always_defrag,
        );
        self.defrag.in_defrag()
    }

    /// Get work packet scheduler
    pub fn scheduler(&self) -> &GCWorkScheduler<VM> {
        &self.scheduler
    }

    fn schedule_defrag_selection_packets(&self, _pause: Pause) {
        self.evac_set.schedule_defrag_selection_packets(self)
    }

    pub fn rc_eager_prepare(&self, pause: Pause) {
        self.block_allocation.notify_mutator_phase_end();
        if pause == Pause::Full || pause == Pause::InitialMark {
            // Update mark_state
            // if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_on_side() {
            //     self.mark_state = Self::MARKED_STATE;
            // } else {
            //     // For header metadata, we use cyclic mark bits.
            //     unimplemented!("cyclic mark bits is not supported at the moment");
            // }
            // Reset block mark and object mark table.
            let work_packets = self.generate_lxr_full_trace_prepare_tasks();
            self.scheduler().work_buckets[WorkBucketStage::Initial].bulk_add(work_packets);
        }
    }

    pub fn schedule_mark_table_zeroing_tasks(&self, stage: WorkBucketStage) {
        let work_packets = self.generate_concurrent_mark_table_zeroing_tasks();
        self.scheduler().work_buckets[stage].bulk_add(work_packets);
    }

    pub fn prepare_rc(&mut self, pause: Pause) {
        self.num_clean_blocks_released_young
            .store(0, Ordering::SeqCst);
        self.num_clean_blocks_released_mature
            .store(0, Ordering::SeqCst);
        self.num_clean_blocks_released_lazy
            .store(0, Ordering::SeqCst);
        self.copy_alloc_bytes.store(0, Ordering::SeqCst);
        self.rc_killed_bytes.store(0, Ordering::SeqCst);
        debug_assert_ne!(pause, Pause::FullDefrag);
        if pause == Pause::InitialMark || pause == Pause::Full {
            // Select mature evacuation set
            if !cfg!(feature = "lxr_no_evac") {
                self.schedule_defrag_selection_packets(pause);
            }
        }
        // Initialize mark state for tracing
        if pause == Pause::Full || pause == Pause::InitialMark {
            // Update mark_state
            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_on_side() {
                self.mark_state = Self::MARKED_STATE;
            } else {
                // For header metadata, we use cyclic mark bits.
                unimplemented!("cyclic mark bits is not supported at the moment");
            }
        }
        // Release nursery blocks
        if pause != Pause::RefCount {
            if pause == Pause::Full {
                // Reset worker TLABs.
                // The block of the current worker TLAB may be selected as part of the mature evacuation set.
                // So the copied mature objects might be copied into defrag blocks, and get copied out again.
                crate::scheduler::worker::reset_workers::<VM>();
            }
            // Release young blocks to reduce to-space overflow
            // self.block_allocation
            //     .sweep_nursery_blocks(&self.scheduler, pause);
            self.flush_page_resource();
        }
        self.block_allocation
            .reset_block_mark_for_mutator_reused_blocks(pause);
        if pause == Pause::FinalMark {
            crate::REMSET_RECORDING.store(false, Ordering::SeqCst);
            self.is_end_of_satb_or_full_gc = true;
        } else if pause == Pause::Full {
            self.is_end_of_satb_or_full_gc = true;
        }
    }

    pub fn release_rc(&mut self, pause: Pause) {
        debug_assert_ne!(pause, Pause::FullDefrag);
        self.block_allocation
            .sweep_nursery_blocks(&self.scheduler, pause);
        self.block_allocation
            .sweep_mutator_reused_blocks(&self.scheduler, pause);
        self.flush_page_resource();
        let disable_lasy_dec_for_current_gc = crate::disable_lasy_dec_for_current_gc();
        if disable_lasy_dec_for_current_gc {
            self.scheduler().process_lazy_decrement_packets();
        } else {
            debug_assert_ne!(pause, Pause::Full);
        }
        self.rc.reset_inc_buffer_size();
        self.is_end_of_satb_or_full_gc = false;
        // This cannot be done in parallel in a separate thread
        self.schedule_mature_sweeping(pause);
        self.reused_lines_consumed.store(0, Ordering::Relaxed);
    }

    pub fn schedule_mature_sweeping(&self, pause: Pause) {
        if pause == Pause::Full || pause == Pause::FinalMark {
            self.evac_set.sweep_mature_evac_candidates(self);
            let disable_lasy_dec_for_current_gc = crate::disable_lasy_dec_for_current_gc();
            let dead_cycle_sweep_packets = self.generate_dead_cycle_sweep_tasks();
            let sweep_los = RCSweepMatureAfterSATBLOS::new(LazySweepingJobsCounter::new_decs());
            if crate::args::LAZY_DECREMENTS && !disable_lasy_dec_for_current_gc {
                debug_assert_ne!(pause, Pause::Full);
                self.scheduler().postpone_all(dead_cycle_sweep_packets);
                self.scheduler().postpone(sweep_los);
            } else {
                self.scheduler().work_buckets[WorkBucketStage::STWRCDecsAndSweep]
                    .bulk_add(dead_cycle_sweep_packets);
                self.scheduler().work_buckets[WorkBucketStage::STWRCDecsAndSweep].add(sweep_los);
            }
        }
    }

    pub(crate) fn prepare(
        &mut self,
        major_gc: bool,
        initial_mark_pause: bool,
        plan_stats: Option<StatsForDefrag>,
        unlog_bits_op: UnlogBitsOperation,
    ) {
        self.initial_mark_pause = initial_mark_pause;
        debug_assert!(!self.rc_enabled);
        if major_gc {
            // Update mark_state
            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_on_side() {
                self.mark_state = Self::MARKED_STATE;
            } else {
                // For header metadata, we use cyclic mark bits.
                unimplemented!("cyclic mark bits is not supported at the moment");
            }

            // Prepare defrag info
            if self.is_defrag_enabled() {
                self.defrag.prepare(self, plan_stats.unwrap());
            }

            // Prepare each block for GC
            let threshold = self.defrag.defrag_spill_threshold.load(Ordering::Acquire);
            // # Safety: ImmixSpace reference is always valid within this collection cycle.
            let space = unsafe { &*(self as *const Self) };
            let work_packets = self.chunk_map.generate_tasks_batched(|chunks| {
                Box::new(PrepareBlockState {
                    space,
                    chunks,
                    defrag_threshold: if space.in_defrag() {
                        Some(threshold)
                    } else {
                        None
                    },
                    unlog_bits_op,
                })
            });
            self.scheduler().work_buckets[WorkBucketStage::Prepare].bulk_add(work_packets);

            if !super::BLOCK_ONLY {
                self.line_mark_state.fetch_add(1, Ordering::AcqRel);
                if self.line_mark_state.load(Ordering::Acquire) > Line::MAX_MARK_STATE {
                    self.line_mark_state
                        .store(Line::RESET_MARK_STATE, Ordering::Release);
                }
            }
        }

        #[cfg(feature = "vo_bit")]
        if vo_bit::helper::need_to_clear_vo_bits_before_tracing::<VM>() {
            let maybe_scope = if major_gc {
                // If it is major GC, we always clear all VO bits because we are doing full-heap
                // tracing.
                Some(VOBitsClearingScope::FullGC)
            } else if self.space_args.mixed_age {
                // StickyImmix nursery GC.
                // Some lines (or blocks) contain only young objects,
                // while other lines (or blocks) contain only old objects.
                if super::BLOCK_ONLY {
                    // Block only.  Young objects are only allocated into fully empty blocks.
                    // Only clear unmarked blocks.
                    Some(VOBitsClearingScope::BlockOnly)
                } else {
                    // Young objects are allocated into empty lines.
                    // Only clear unmarked lines.
                    let line_mark_state = self.line_mark_state.load(Ordering::SeqCst);
                    Some(VOBitsClearingScope::Line {
                        state: line_mark_state,
                    })
                }
            } else {
                // GenImmix nursery GC.  We do nothing to the ImmixSpace because the nursery is a
                // separate CopySpace.  It'll clear its own VO bits.
                None
            };

            if let Some(scope) = maybe_scope {
                let work_packets = self
                    .chunk_map
                    .generate_tasks(|chunk| Box::new(ClearVOBitsAfterPrepare { chunk, scope }));
                self.scheduler.work_buckets[WorkBucketStage::ClearVOBits].bulk_add(work_packets);
            }
        }
    }

    /// Release for the immix space.
    pub(crate) fn release(&mut self, major_gc: bool, unlog_bits_op: UnlogBitsOperation) {
        debug_assert!(!self.rc_enabled);
        if major_gc {
            // Update line_unavail_state for hole searching after this GC.
            if !super::BLOCK_ONLY {
                self.line_unavail_state.store(
                    self.line_mark_state.load(Ordering::Acquire),
                    Ordering::Release,
                );
            }
        }
        // Clear reusable blocks list
        if !super::BLOCK_ONLY {
            self.reusable_blocks.reset();
        }
        // Sweep chunks and blocks
        let work_packets = self.generate_sweep_tasks(unlog_bits_op);
        self.scheduler().work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
        self.initial_mark_pause = false;

        self.lines_consumed.store(0, Ordering::Relaxed);
    }

    /// This is called when a GC finished.
    /// Return whether this GC was a defrag GC, as a plan may want to know this.
    pub fn end_of_gc(&mut self) -> bool {
        let did_defrag = self.defrag.in_defrag();
        if self.is_defrag_enabled() {
            self.defrag.reset_in_defrag();
        }
        did_defrag
    }

    /// Generate chunk sweep tasks
    fn generate_sweep_tasks(&self, unlog_bits_op: UnlogBitsOperation) -> Vec<Box<dyn GCWork<VM>>> {
        self.defrag.mark_histograms.lock().clear();
        // # Safety: ImmixSpace reference is always valid within this collection cycle.
        let space = unsafe { &*(self as *const Self) };
        let epilogue = Arc::new(FlushPageResource {
            space,
            counter: AtomicUsize::new(0),
        });
        let tasks = self.chunk_map.generate_tasks_batched(|chunks| {
            Box::new(SweepChunk {
                space,
                chunks,
                unlog_bits_op,
                epilogue: epilogue.clone(),
            })
        });
        epilogue.counter.store(tasks.len(), Ordering::SeqCst);
        tasks
    }

    /// Generate chunk sweep work packets.
    pub fn generate_dead_cycle_sweep_tasks(&self) -> Vec<Box<dyn GCWork<VM>>> {
        self.chunk_map.generate_tasks_batched(|chunks| {
            Box::new(SweepDeadCycles::new(
                chunks,
                LazySweepingJobsCounter::new_decs(),
            ))
        })
    }

    /// Generate chunk sweep work packets.
    fn generate_lxr_full_trace_prepare_tasks(&self) -> Vec<Box<dyn GCWork<VM>>> {
        assert!(self.rc_enabled);
        self.chunk_map
            .generate_tasks_batched(|chunks| Box::new(PrepareChunksForFullGC { chunks }))
    }

    pub fn generate_concurrent_mark_table_zeroing_tasks(&self) -> Vec<Box<dyn GCWork<VM>>> {
        self.chunk_map
            .generate_tasks_batched(|chunks| Box::new(ConcurrentChunkMetadataZeroing { chunks }))
    }

    /// Release a block.
    pub fn release_block(
        &self,
        block: Block,
        nursery: bool,
        zero_unlog_table: bool,
        single_thread: bool,
    ) {
        if crate::verbose(2) {
            if nursery {
                RELEASED_NURSERY_BLOCKS.fetch_add(1, Ordering::SeqCst);
            }
            RELEASED_BLOCKS.fetch_add(1, Ordering::SeqCst);
        }
        if crate::plan::barriers::BARRIER_MEASUREMENT || zero_unlog_table {
            block.clear_field_unlog_table::<VM>();
        }
        block.deinit(self);
        self.pr.release_block(block, single_thread);
    }

    /// Allocate a clean block.
    pub fn get_clean_block(
        &self,
        tls: VMThread,
        copy: bool,
        alloc_options: AllocationOptions,
    ) -> Option<Block> {
        let block_address = self.acquire(tls, Block::PAGES, alloc_options);
        if block_address.is_zero() {
            return None;
        }
        if !self.rc_enabled {
            self.defrag.notify_new_clean_block(copy);
        }
        let block = Block::from_aligned_address(block_address);
        if !copy && self.rc_enabled {
            self.block_allocation.nursery_blocks.push(block);
        }
        self.block_allocation
            .initialize_new_clean_block(block, copy, self.cm_enabled);
        self.chunk_map.set_allocated(block.chunk(), true);
        if !self.rc_enabled {
            self.lines_consumed
                .fetch_add(Block::LINES, Ordering::SeqCst);
        }

        Some(block)
    }

    /// Pop a reusable block from the reusable block list.
    pub fn get_reusable_block(&self, copy: bool) -> Option<Block> {
        if super::BLOCK_ONLY {
            return None;
        }
        loop {
            if let Some(block) = self.reusable_blocks.pop() {
                // Skip blocks that should be evacuated.
                if copy && block.is_defrag_source() {
                    continue;
                }
                if self.rc_enabled {
                    if crate::args::RC_MATURE_EVACUATION && block.is_defrag_source() {
                        continue;
                    }
                    // Blocks in the `reusable_blocks` queue can be released after some RC collections.
                    // These blocks can either have `Unallocated` state, or be reallocated again.
                    // Skip these cases and only return the truly reusable blocks.
                    if !block.get_state().is_reusable() {
                        continue;
                    }
                    if !copy && !block.attempt_mutator_reuse() {
                        continue;
                    }
                    if !copy {
                        self.block_allocation.reused_blocks.push(block);
                    }
                } else {
                    // Get available lines. Do this before block.init which will reset block state.
                    let lines_delta = match block.get_state() {
                        BlockState::Reusable { unavailable_lines } => {
                            Block::LINES - unavailable_lines as usize
                        }
                        BlockState::Unmarked => Block::LINES,
                        _ => unreachable!("{:?} {:?}", block, block.get_state()),
                    };
                    self.lines_consumed.fetch_add(lines_delta, Ordering::SeqCst);
                }

                block.init(copy, true, self);
                return Some(block);
            } else {
                return None;
            }
        }
    }

    /// Trace and mark objects without evacuation.
    pub fn process_mature_evacuation_remset(&self) {
        let mut remsets = vec![];
        mem::swap(&mut remsets, &mut self.mature_evac_remsets.lock().unwrap());
        self.scheduler.work_buckets[WorkBucketStage::RCEvacuateMature].bulk_add(remsets);
    }

    pub fn trace_object_without_moving_rc(
        &self,
        queue: &mut impl ObjectQueue,
        object: ObjectReference,
    ) -> ObjectReference {
        if self.attempt_mark(object) {
            let addr = object.to_raw_address().as_usize();
            let straddle = if (addr & 0b11110000) == 0 {
                self.rc
                    .is_straddle_line(Line::from(Line::align(object.to_raw_address())))
            } else {
                false
            };

            // let straddle = self
            //     .rc
            //     .is_straddle_line(Line::from(Line::align(object.to_raw_address())));
            if !straddle {
                // Visit node
                queue.enqueue(object);
            }
        }
        object
    }

    /// Trace and mark objects without evacuation.
    pub fn trace_object_without_moving(
        &self,
        queue: &mut impl ObjectQueue,
        object: ObjectReference,
    ) -> ObjectReference {
        #[cfg(feature = "vo_bit")]
        vo_bit::helper::on_trace_object::<VM>(object);

        if self.attempt_mark(object) {
            if self.rc_enabled {
                let straddle = self
                    .rc
                    .is_straddle_line(Line::from(Line::align(object.to_raw_address())));
                if straddle {
                    return object;
                }
            } else {
                // Mark block and lines
                if !super::BLOCK_ONLY {
                    if !super::MARK_LINE_AT_SCAN_TIME {
                        self.mark_lines(object);
                    }
                } else {
                    let block = Block::containing(object);
                    let state = block.get_state();
                    if state != BlockState::Nursery && state != BlockState::Marked {
                        block.set_state(BlockState::Marked);
                    }
                }
            }

            #[cfg(feature = "vo_bit")]
            vo_bit::helper::on_object_marked::<VM>(object);

            // Visit node
            queue.enqueue(object);
            if !self.rc_enabled {
                self.unlog_object_if_needed(object);
            }
            return object;
        }
        object
    }

    /// Trace object and do evacuation if required.
    #[allow(clippy::assertions_on_constants)]
    pub fn trace_object_with_opportunistic_copy(
        &self,
        queue: &mut impl ObjectQueue,
        object: ObjectReference,
        semantics: CopySemantics,
        worker: &mut GCWorker<VM>,
        nursery_collection: bool,
    ) -> ObjectReference {
        let copy_context = worker.get_copy_context_mut();
        debug_assert!(!super::BLOCK_ONLY);

        #[cfg(feature = "vo_bit")]
        vo_bit::helper::on_trace_object::<VM>(object);

        let forwarding_status = object_forwarding::attempt_to_forward::<VM>(object);
        if object_forwarding::state_is_forwarded_or_being_forwarded(forwarding_status) {
            // We lost the forwarding race as some other thread has set the forwarding word; wait
            // until the object has been forwarded by the winner. Note that the object may not
            // necessarily get forwarded since Immix opportunistically moves objects.
            #[allow(clippy::let_and_return)]
            let new_object =
                object_forwarding::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
            #[cfg(debug_assertions)]
            {
                if new_object == object {
                    debug_assert!(
                        self.is_marked(object) || self.defrag.space_exhausted() || self.is_pinned(object),
                        "Forwarded object is the same as original object {} even though it should have been copied",
                        object,
                    );
                } else {
                    // new_object != object
                    debug_assert!(
                        !Block::containing(new_object).is_defrag_source(),
                        "Block {:?} containing forwarded object {} should not be a defragmentation source",
                        Block::containing(new_object),
                        new_object,
                    );
                }
            }
            new_object
        } else if self.is_marked(object) {
            // We won the forwarding race but the object is already marked so we clear the
            // forwarding status and return the unmoved object
            object_forwarding::clear_forwarding_bits::<VM>(object);
            object
        } else {
            // We won the forwarding race; actually forward and copy the object if it is not pinned
            // and we have sufficient space in our copy allocator
            debug_assert!(!nursery_collection || !self.rc_enabled);
            let new_object = if self.is_pinned(object)
                || (!nursery_collection && self.defrag.space_exhausted())
            {
                self.attempt_mark(object);
                object_forwarding::clear_forwarding_bits::<VM>(object);
                Block::containing(object).set_state(BlockState::Marked);

                #[cfg(feature = "vo_bit")]
                vo_bit::helper::on_object_marked::<VM>(object);

                if !super::MARK_LINE_AT_SCAN_TIME {
                    self.mark_lines(object);
                }

                self.unlog_object_if_needed(object);

                object
            } else {
                // We are forwarding objects. When the copy allocator allocates the block, it should
                // mark the block. So we do not need to explicitly mark it here.
                // Clippy complains if the "vo_bit" feature is not enabled.
                #[allow(clippy::let_and_return)]
                let new_object =
                    object_forwarding::try_forward_object::<VM>(object, semantics, copy_context)
                        .expect("to-space overflow");

                #[cfg(feature = "vo_bit")]
                vo_bit::helper::on_object_forwarded::<VM>(new_object);

                new_object
            };
            debug_assert!({
                let state = Block::containing(new_object).get_state();
                state == BlockState::Marked || state == BlockState::Nursery
            });

            queue.enqueue(new_object);
            debug_assert!(new_object.is_live());
            new_object
        }
    }

    pub fn rc_trace_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        semantics: CopySemantics,
        pause: Pause,
        mark: bool,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        debug_assert!(self.rc_enabled);
        if crate::args::RC_MATURE_EVACUATION && Block::containing(object).is_defrag_source() {
            self.trace_forward_rc_mature_object(queue, object, semantics, pause, worker)
        } else if crate::args::RC_MATURE_EVACUATION {
            self.trace_mark_rc_mature_object(queue, object, pause, mark)
        } else {
            self.trace_object_without_moving(queue, object)
        }
    }

    pub fn trace_mark_rc_mature_object(
        &self,
        queue: &mut impl ObjectQueue,
        object: ObjectReference,
        _pause: Pause,
        mark: bool,
    ) -> ObjectReference {
        debug_assert!(
            !object_forwarding::is_forwarded::<VM>(object),
            "object {:?} is forwarded",
            object
        );
        if mark && self.attempt_mark(object) {
            queue.enqueue(object);
        }
        object
    }

    #[allow(clippy::assertions_on_constants)]
    pub fn trace_forward_rc_mature_object<Q: ObjectQueue>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        _semantics: CopySemantics,
        _pause: Pause,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        let copy_context = worker.get_copy_context_mut();
        let forwarding_status = object_forwarding::attempt_to_forward::<VM>(object);
        if object_forwarding::state_is_forwarded_or_being_forwarded(forwarding_status) {
            let new =
                object_forwarding::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
            new
        } else {
            // Evacuate the mature object
            let new = object_forwarding::try_forward_object::<VM>(
                object,
                CopySemantics::DefaultCopy,
                copy_context,
            )
            .expect("to-space overflow");
            // Transfer RC count
            if new.get_size::<VM>() > Line::BYTES {
                self.rc.mark_straddle_object(new);
            }
            self.rc.set(new, self.rc.count(object));
            self.attempt_mark(new);
            self.unmark(object);
            queue.enqueue(new);
            debug_assert_ne!(
                self.rc.count(new),
                0,
                "ERROR Invalid {:?} rc={}",
                new,
                self.rc.count(new)
            );
            new
        }
    }

    fn unlog_object_if_needed(&self, object: ObjectReference) {
        debug_assert!(!self.rc_enabled);
        if self.common.unlog_traced_object {
            // Make sure the side metadata for the line can fit into one byte. For smaller line size, we should
            // use `mark_as_unlogged` instead to mark the bit.
            // const_assert!(
            //     Line::BYTES
            //         >= (1
            //             << (crate::util::constants::LOG_BITS_IN_BYTE
            //                 + crate::util::constants::LOG_MIN_OBJECT_SIZE))
            // );
            // const_assert_eq!(
            //     crate::vm::object_model::specs::VMGlobalLogBitSpec::LOG_NUM_BITS,
            //     0
            // ); // We should put this to the addition, but type casting is not allowed in constant assertions.

            // Every immix line is 256 bytes, which is mapped to 4 bytes in the side metadata.
            // If we have one object in the line that is mature, we can assume all the objects in the line are mature objects.
            // So we can just mark the byte.
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                .mark_byte_as_unlogged::<VM>(object, Ordering::Relaxed);
        }
    }

    /// Mark all the lines that the given object spans.
    #[allow(clippy::assertions_on_constants)]
    pub fn mark_lines(&self, object: ObjectReference) {
        debug_assert!(!super::BLOCK_ONLY);
        if self.rc_enabled {
            return;
        }
        Line::mark_lines_for_object::<VM>(object, self.line_mark_state.load(Ordering::Acquire));
    }

    /// Atomically mark an object.
    pub fn attempt_mark(&self, object: ObjectReference) -> bool {
        let result = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.fetch_update_metadata::<VM, u8, _>(
            object,
            Ordering::Relaxed,
            Ordering::Relaxed,
            |v| {
                if v != 0 {
                    return None;
                }
                Some(1)
            },
        );
        result.is_ok()
    }

    /// Atomically mark an object.
    pub fn unmark(&self, object: ObjectReference) -> bool {
        let result = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.fetch_update_metadata::<VM, u8, _>(
            object,
            Ordering::Relaxed,
            Ordering::Relaxed,
            |v| {
                if v != 1 {
                    return None;
                }
                Some(0)
            },
        );
        result.is_ok()
    }

    pub fn is_marked(&self, object: ObjectReference) -> bool {
        let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::Relaxed,
        );
        old_value == 1
    }

    pub fn line_is_marked(&self, a: Address) -> bool {
        let b = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
            .extract_side_spec()
            .load_byte(a);
        b == u8::MAX
    }

    /// Check if an object is pinned.
    fn is_pinned(&self, _object: ObjectReference) -> bool {
        #[cfg(feature = "object_pinning")]
        return self.is_object_pinned(_object);

        #[cfg(not(feature = "object_pinning"))]
        false
    }

    /// Hole searching.
    ///
    /// Linearly scan lines in a block to search for the next
    /// hole, starting from the given line. If we find available lines,
    /// return a tuple of the start line and the end line (non-inclusive).
    ///
    /// Returns None if the search could not find any more holes.
    #[allow(clippy::assertions_on_constants)]
    pub fn get_next_available_lines(&self, copy: bool, search_start: Line) -> Option<(Line, Line)> {
        debug_assert!(!super::BLOCK_ONLY);
        if self.rc_enabled {
            self.rc_get_next_available_lines(copy, search_start)
        } else {
            self.normal_get_next_available_lines(copy, search_start)
        }
    }

    /// Search holes by ref-counts instead of line marks
    #[allow(clippy::assertions_on_constants)]
    pub fn rc_get_next_available_lines(
        &self,
        copy: bool,
        search_start: Line,
    ) -> Option<(Line, Line)> {
        debug_assert!(!super::BLOCK_ONLY);
        debug_assert!(self.rc_enabled);
        let block = search_start.block();
        let rc_array = RCArray::of(block);
        let limit = Block::LINES;
        // Find start
        let first_free_cursor = {
            let start_cursor = search_start.get_index_within_block();
            let mut first_free_cursor = None;
            let mut find_free_line = false;
            for i in start_cursor..limit {
                if rc_array.is_dead(i) {
                    if i == 0 {
                        first_free_cursor = Some(i);
                        break;
                    } else if !find_free_line {
                        find_free_line = true;
                    } else {
                        first_free_cursor = Some(i);
                        break;
                    }
                } else {
                    find_free_line = false;
                }
            }
            first_free_cursor
        };
        let start = match first_free_cursor {
            Some(c) => c,
            _ => return None,
        };
        // Find limit
        let end = {
            let mut cursor = start + 1;
            while cursor < limit {
                if !rc_array.is_dead(cursor) {
                    break;
                }
                cursor += 1;
            }
            cursor
        };
        let start = Line::from(block.start() + (start << Line::LOG_BYTES));
        let end = Line::from(block.start() + (end << Line::LOG_BYTES));
        if Line::steps_between(&start, &end).unwrap() < crate::args().min_reuse_lines {
            if end == block.end_line() {
                return None;
            } else {
                return self.rc_get_next_available_lines(copy, end);
            };
        }
        if self.common.needs_log_bit {
            if !copy {
                Line::clear_field_unlog_table::<VM>(start..end);
            } else {
                Line::initialize_field_unlog_table_as_unlogged::<VM>(start..end);
            }
        }
        let num_lines = Line::steps_between(&start, &end).unwrap();
        if !copy {
            self.reused_lines_consumed
                .fetch_add(num_lines, Ordering::Relaxed);
        }
        block.dec_dead_bytes_sloppy((num_lines as u32) << Line::LOG_BYTES);
        if self.block_allocation.cm_in_progress_or_final_mark() {
            Line::initialize_mark_table_as_marked::<VM>(start..end);
            Line::inc_reuse_counts::<VM>(start..end);
        } else {
            // Line::clear_mark_table::<VM>(start..end);
        }
        // if !_copy {
        //     println!("reuse {:?} copy={}", start..end, copy);
        // }
        Some((start, end))
    }

    #[allow(clippy::assertions_on_constants)]
    pub fn normal_get_next_available_lines(
        &self,
        copy: bool,
        search_start: Line,
    ) -> Option<(Line, Line)> {
        debug_assert!(!super::BLOCK_ONLY);
        debug_assert!(!self.rc_enabled);
        let unavail_state = self.line_unavail_state.load(Ordering::Acquire);
        let current_state = self.line_mark_state.load(Ordering::Acquire);
        let block = search_start.block();
        let mut mark_data = block.line_mark_table();
        let start_cursor = search_start.get_index_within_block();
        let mut cursor = start_cursor;
        // Find start
        while cursor < Block::LINES {
            let mark = mark_data.get(cursor);
            if mark != unavail_state && mark != current_state {
                break;
            }
            cursor += 1;
        }
        if cursor == Block::LINES {
            return None;
        }
        let start = search_start.next_nth(cursor - start_cursor);
        // Find limit
        while cursor < Block::LINES {
            let mark = mark_data.get(cursor);
            if mark == unavail_state || mark == current_state {
                break;
            }
            if self.cm_enabled {
                mark_data.set(cursor, current_state);
            }
            cursor += 1;
        }
        let end = search_start.next_nth(cursor - start_cursor);
        if Line::steps_between(&start, &end).unwrap() < crate::args().min_reuse_lines {
            if end == block.end_line() {
                return None;
            } else {
                return self.normal_get_next_available_lines(copy, end);
            };
        }
        if self.common.needs_log_bit {
            if !copy {
                Line::clear_field_unlog_table::<VM>(start..end);
            } else {
                Line::initialize_field_unlog_table_as_unlogged::<VM>(start..end);
            }
        }
        Some((start, end))
    }

    pub fn is_last_gc_exhaustive(&self, did_defrag_for_last_gc: bool) -> bool {
        if self.is_defrag_enabled() {
            did_defrag_for_last_gc
        } else {
            // If defrag is disabled, every GC is exhaustive.
            true
        }
    }

    pub fn add_to_possibly_dead_mature_blocks(&self, block: Block, is_defrag_source: bool) {
        if block.log() {
            self.possibly_dead_mature_blocks
                .push((block, is_defrag_source));
        }
    }

    pub fn schedule_rc_block_sweeping_tasks(&self, counter: LazySweepingJobsCounter) {
        // while let Some(x) = self.last_mutator_recycled_blocks.pop() {
        //     x.set_state(BlockState::Marked);
        // }
        // This may happen either within a pause, or in concurrent.
        let size = self.possibly_dead_mature_blocks.len();
        let num_bins = self.scheduler().num_workers();
        let bin_cap = size / num_bins + if size % num_bins == 0 { 0 } else { 1 };
        let mut bins = (0..num_bins)
            .map(|_| Vec::with_capacity(bin_cap))
            .collect::<Vec<Vec<(Block, bool)>>>();
        'out: for i in 0..num_bins {
            for _ in 0..bin_cap {
                if let Some(block) = self.possibly_dead_mature_blocks.pop() {
                    bins[i].push(block);
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
        self.scheduler().work_buckets[WorkBucketStage::Unconstrained].bulk_add_prioritized(packets);
    }

    pub(crate) fn get_mutator_recycled_lines_in_pages(&self) -> usize {
        debug_assert!(self.rc_enabled);
        self.reused_lines_consumed.load(Ordering::Relaxed)
            >> (LOG_BYTES_IN_PAGE - Line::LOG_BYTES as u8)
    }

    pub(crate) fn get_pages_allocated(&self) -> usize {
        debug_assert!(!self.rc_enabled);
        self.lines_consumed.load(Ordering::Relaxed) >> (LOG_BYTES_IN_PAGE - Line::LOG_BYTES as u8)
    }

    /// Post copy routine for Immix copy contexts
    fn post_copy(&self, object: ObjectReference, _bytes: usize) {
        if self.rc_enabled {
            return;
        }
        // Mark the object
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(
            object,
            self.mark_state,
            None,
            Ordering::SeqCst,
        );
        // Mark the line
        if !super::MARK_LINE_AT_SCAN_TIME {
            self.mark_lines(object);
        }
        if self.common.unlog_traced_object {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC
                .mark_byte_as_unlogged::<VM>(object, Ordering::Relaxed);
        }
    }

    pub(crate) fn prefer_copy_on_nursery_gc(&self) -> bool {
        self.is_nursery_copy_enabled()
    }

    pub(crate) fn is_nursery_copy_enabled(&self) -> bool {
        !self.space_args.never_move_objects && !cfg!(feature = "sticky_immix_non_moving_nursery")
    }

    pub(crate) fn is_defrag_enabled(&self) -> bool {
        !self.space_args.never_move_objects
    }
}

/// A work packet to prepare each block for a major GC.
/// Performs the action on a range of chunks.
pub struct PrepareBlockState<VM: VMBinding> {
    #[allow(dead_code)]
    pub space: &'static ImmixSpace<VM>,
    pub chunks: Range<Chunk>,
    pub defrag_threshold: Option<usize>,
    pub unlog_bits_op: UnlogBitsOperation,
}
impl<VM: VMBinding> PrepareBlockState<VM> {
    /// Clear object mark table
    fn reset_object_mark(&self, chunk: Chunk) {
        // NOTE: We reset the mark bits because cyclic mark bit is currently not supported, yet.
        // See `ImmixSpace::prepare`.
        if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
            side.bzero_metadata(chunk.start(), Chunk::BYTES);
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for PrepareBlockState<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let num_chunks = (self.chunks.end.start() - self.chunks.start.start()) >> Chunk::LOG_BYTES;
        let defrag_every_block = *mmtk.options.immix_defrag_every_block;
        for i in 0..num_chunks {
            let chunk = self.chunks.start.next_nth(i);
            if !self.space.chunk_map.is_allocated(chunk) {
                continue;
            }
            // Clear object mark table for this chunk
            self.reset_object_mark(chunk);
            // Iterate over all blocks in this chunk
            for block in chunk.iter_region::<Block>() {
                let state = block.get_state();
                // Skip unallocated blocks.
                if state == BlockState::Unallocated {
                    continue;
                }
                // Check if this block needs to be defragmented.
                let is_defrag_source = if !self.space.is_defrag_enabled() {
                    // Do not set any block as defrag source if defrag is disabled.
                    false
                } else if defrag_every_block {
                    // Set every block as defrag source if so desired.
                    true
                } else if let Some(defrag_threshold) = self.defrag_threshold {
                    // This GC is a defrag GC.
                    block.get_holes() >= defrag_threshold
                } else {
                    // Not a defrag GC.
                    false
                };
                block.set_as_defrag_source(is_defrag_source);
                // Clear block mark data.
                block.set_state(BlockState::Unmarked);
                self.unlog_bits_op
                    .execute::<VM>(block.start(), Block::BYTES);
                debug_assert!(!block.get_state().is_reusable());
                debug_assert_ne!(block.get_state(), BlockState::Marked);
            }
        }
    }
}

/// Chunk sweeping work packet.
struct SweepChunk<VM: VMBinding> {
    space: &'static ImmixSpace<VM>,
    chunks: Range<Chunk>,
    unlog_bits_op: UnlogBitsOperation,
    /// A destructor invoked when all `SweepChunk` packets are finished.
    epilogue: Arc<FlushPageResource<VM>>,
}

impl<VM: VMBinding> GCWork<VM> for SweepChunk<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let mut histogram = self.space.defrag.new_histogram();
        let num_chunks = (self.chunks.end.start() - self.chunks.start.start()) >> Chunk::LOG_BYTES;
        for i in 0..num_chunks {
            let chunk = self.chunks.start.next_nth(i);
            if !self.space.chunk_map.is_allocated(chunk) {
                continue;
            }
            let line_mark_state = if super::BLOCK_ONLY {
                None
            } else {
                Some(self.space.line_mark_state.load(Ordering::Acquire))
            };
            // Hints for clearing side forwarding bits.
            let is_moving_gc = mmtk.get_plan().current_gc_may_move_object();
            let is_defrag_gc = self.space.defrag.in_defrag();
            // number of allocated blocks.
            let mut allocated_blocks = 0;
            // Iterate over all allocated blocks in this chunk.
            for block in chunk
                .iter_region::<Block>()
                .filter(|block| block.get_state() != BlockState::Unallocated)
            {
                // Clear side forwarding bits.
                // In the beginning of the next GC, no side forwarding bits shall be set.
                // In this way, we can omit clearing forwarding bits when copying object.
                // See `GCWorkerCopyContext::post_copy`.
                // Note, `block.sweep()` overwrites `DEFRAG_STATE_TABLE` with the number of holes,
                // but we need it to know if a block is a defrag source.
                // We clear forwarding bits before `block.sweep()`.
                if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_FORWARDING_BITS_SPEC {
                    if is_moving_gc {
                        let objects_may_move = if is_defrag_gc {
                            // If it is a defrag GC, we only clear forwarding bits for defrag sources.
                            block.is_defrag_source()
                        } else {
                            // Otherwise, it must be a nursery GC of StickyImmix with copying nursery.
                            // We don't have information about which block contains moved objects,
                            // so we have to clear forwarding bits for all blocks.
                            true
                        };
                        if objects_may_move {
                            side.bzero_metadata(block.start(), Block::BYTES);
                        }
                    }
                }

                self.unlog_bits_op
                    .execute::<VM>(block.start(), Block::BYTES);

                if !block.sweep(self.space, &mut histogram, line_mark_state) {
                    // Block is live. Increment the allocated block count.
                    allocated_blocks += 1;
                }
            }
            probe!(mmtk, sweep_chunk, allocated_blocks);
            // Set this chunk as free if there is not live blocks.
            if allocated_blocks == 0 {
                self.space.chunk_map.set_allocated(chunk, false);
            }
        }
        self.space.defrag.add_completed_mark_histogram(histogram);

        self.epilogue.finish_one_work_packet();
    }
}

/// Count number of remaining work pacets, and flush page resource if all packets are finished.
struct FlushPageResource<VM: VMBinding> {
    space: &'static ImmixSpace<VM>,
    counter: AtomicUsize,
}

impl<VM: VMBinding> FlushPageResource<VM> {
    /// Called after a related work packet is finished.
    fn finish_one_work_packet(&self) {
        if 1 == self.counter.fetch_sub(1, Ordering::SeqCst) {
            // We've finished releasing all the dead blocks to the BlockPageResource's thread-local queues.
            // Now flush the BlockPageResource.
            self.space.flush_page_resource()
        }
    }
}

impl<VM: VMBinding> Drop for FlushPageResource<VM> {
    fn drop(&mut self) {
        epilogue::debug_assert_counter_zero(&self.counter, "FlushPageResource::counter");
    }
}

use crate::policy::copy_context::PolicyCopyContext;
use crate::util::alloc::Allocator;
use crate::util::alloc::ImmixAllocator;

/// Normal immix copy context. It has one copying Immix allocator.
/// Most immix plans use this copy context.
pub struct ImmixCopyContext<VM: VMBinding> {
    allocator: ImmixAllocator<VM>,
}

impl<VM: VMBinding> PolicyCopyContext for ImmixCopyContext<VM> {
    type VM = VM;

    fn prepare(&mut self) {
        self.allocator.reset();
    }
    fn release(&mut self) {
        self.allocator.reset();
    }
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: usize,
    ) -> Address {
        self.allocator.alloc(bytes, align, offset)
    }
    fn post_copy(&mut self, obj: ObjectReference, bytes: usize) {
        self.get_space().post_copy(obj, bytes)
    }
}

impl<VM: VMBinding> ImmixCopyContext<VM> {
    pub(crate) fn new(
        tls: VMWorkerThread,
        context: Arc<AllocatorContext<VM>>,
        space: &'static ImmixSpace<VM>,
    ) -> Self {
        ImmixCopyContext {
            allocator: ImmixAllocator::new(tls.0, Some(space), context, true),
        }
    }

    fn get_space(&self) -> &ImmixSpace<VM> {
        self.allocator.immix_space()
    }
}

/// Hybrid Immix copy context. It includes two different immix allocators. One with `copy = true`
/// is used for defrag GCs, and the other is used for other purposes (such as promoting objects from
/// nursery to Immix mature space). This is used by generational immix.
pub struct ImmixHybridCopyContext<VM: VMBinding> {
    copy_allocator: ImmixAllocator<VM>,
    defrag_allocator: ImmixAllocator<VM>,
}

impl<VM: VMBinding> PolicyCopyContext for ImmixHybridCopyContext<VM> {
    type VM = VM;

    fn prepare(&mut self) {
        self.copy_allocator.reset();
        self.defrag_allocator.reset();
    }
    fn release(&mut self) {
        self.copy_allocator.reset();
        self.defrag_allocator.reset();
    }
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: usize,
    ) -> Address {
        if self.get_space().in_defrag() {
            self.defrag_allocator.alloc(bytes, align, offset)
        } else {
            self.copy_allocator.alloc(bytes, align, offset)
        }
    }
    fn post_copy(&mut self, obj: ObjectReference, bytes: usize) {
        self.get_space().post_copy(obj, bytes)
    }
}

impl<VM: VMBinding> ImmixHybridCopyContext<VM> {
    pub(crate) fn new(
        tls: VMWorkerThread,
        context: Arc<AllocatorContext<VM>>,
        space: &'static ImmixSpace<VM>,
    ) -> Self {
        ImmixHybridCopyContext {
            copy_allocator: ImmixAllocator::new(tls.0, Some(space), context.clone(), true),
            defrag_allocator: ImmixAllocator::new(tls.0, Some(space), context, true),
        }
    }

    fn get_space(&self) -> &ImmixSpace<VM> {
        // Both copy allocators should point to the same space.
        debug_assert_eq!(
            self.defrag_allocator.immix_space().common().descriptor,
            self.copy_allocator.immix_space().common().descriptor
        );
        // Just get the space from either allocator
        self.defrag_allocator.immix_space()
    }
}

#[cfg(feature = "vo_bit")]
#[derive(Clone, Copy)]
enum VOBitsClearingScope {
    /// Clear all VO bits in all blocks.
    FullGC,
    /// Clear unmarked blocks, only.
    BlockOnly,
    /// Clear unmarked lines, only.  (i.e. lines with line mark state **not** equal to `state`).
    Line { state: u8 },
}

/// A work packet to clear VO bit metadata after Prepare.
#[cfg(feature = "vo_bit")]
struct ClearVOBitsAfterPrepare {
    chunk: Chunk,
    scope: VOBitsClearingScope,
}

#[cfg(feature = "vo_bit")]
impl<VM: VMBinding> GCWork<VM> for ClearVOBitsAfterPrepare {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        match self.scope {
            VOBitsClearingScope::FullGC => {
                vo_bit::bzero_vo_bit(self.chunk.start(), Chunk::BYTES);
            }
            VOBitsClearingScope::BlockOnly => {
                self.clear_blocks(None);
            }
            VOBitsClearingScope::Line { state } => {
                self.clear_blocks(Some(state));
            }
        }
    }
}

#[cfg(feature = "vo_bit")]
impl ClearVOBitsAfterPrepare {
    fn clear_blocks(&mut self, line_mark_state: Option<u8>) {
        for block in self
            .chunk
            .iter_region::<Block>()
            .filter(|block| block.get_state() != BlockState::Unallocated)
        {
            block.clear_vo_bits_for_unmarked_regions(line_mark_state);
        }
    }
}
