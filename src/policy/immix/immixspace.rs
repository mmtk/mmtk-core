use super::line::*;
use super::{
    block::*,
    chunk::{Chunk, ChunkMap, ChunkState},
    defrag::Defrag,
};
use crate::policy::gc_work::TraceKind;
use crate::policy::sft::GCWorkerMutRef;
use crate::policy::sft::SFT;
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space};
use crate::util::copy::*;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::BlockPageResource;
use crate::util::heap::HeapMeta;
use crate::util::heap::PageResource;
use crate::util::heap::VMRequest;
use crate::util::linear_scan::{Region, RegionIterator};
use crate::util::metadata::side_metadata::{SideMetadataContext, SideMetadataSpec};
use crate::util::metadata::{self, MetadataSpec};
use crate::util::object_forwarding as ForwardingWord;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::{
    plan::ObjectQueue,
    scheduler::{GCWork, GCWorkScheduler, GCWorker, WorkBucketStage},
    util::opaque_pointer::{VMThread, VMWorkerThread},
    MMTK,
};
use atomic::Ordering;
use std::sync::{atomic::AtomicU8, Arc};

pub(crate) const TRACE_KIND_FAST: TraceKind = 0;
pub(crate) const TRACE_KIND_DEFRAG: TraceKind = 1;

pub struct ImmixSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: BlockPageResource<VM, Block>,
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
    /// Object mark state
    mark_state: u8,
    /// Work packet scheduler
    scheduler: Arc<GCWorkScheduler<VM>>,
}

unsafe impl<VM: VMBinding> Sync for ImmixSpace<VM> {}

impl<VM: VMBinding> SFT for ImmixSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        if !super::DEFRAG {
            // If defrag is disabled, we won't forward objects.
            self.is_marked(object, self.mark_state)
        } else {
            self.is_marked(object, self.mark_state) || ForwardingWord::is_forwarded::<VM>(object)
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
        super::DEFRAG
    }

    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_object_metadata(&self, _object: ObjectReference, _alloc: bool) {
        #[cfg(feature = "global_alloc_bit")]
        crate::util::alloc_bit::set_alloc_bit::<VM>(_object);
    }
    #[cfg(feature = "is_mmtk_object")]
    #[inline(always)]
    fn is_mmtk_object(&self, addr: Address) -> bool {
        crate::util::alloc_bit::is_alloced_object::<VM>(addr).is_some()
    }
    #[inline(always)]
    fn sft_trace_object(
        &self,
        _queue: &mut VectorObjectQueue,
        _object: ObjectReference,
        _worker: GCWorkerMutRef,
    ) -> ObjectReference {
        panic!("We do not use SFT to trace objects for Immix. sft_trace_object() cannot be used.")
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
    fn common(&self) -> &CommonSpace<VM> {
        &self.common
    }
    fn initialize_sft(&self) {
        self.common().initialize_sft(self.as_sft())
    }
    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immixspace only releases pages enmasse")
    }
    fn set_copy_for_sft_trace(&mut self, _semantics: Option<CopySemantics>) {
        panic!("We do not use SFT to trace objects for Immix. set_copy_context() cannot be used.")
    }
}

impl<VM: VMBinding> crate::policy::gc_work::PolicyTraceObject<VM> for ImmixSpace<VM> {
    #[inline(always)]
    fn trace_object<Q: ObjectQueue, const KIND: TraceKind>(
        &self,
        queue: &mut Q,
        object: ObjectReference,
        copy: Option<CopySemantics>,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        if KIND == TRACE_KIND_DEFRAG {
            self.trace_object(queue, object, copy.unwrap(), worker)
        } else if KIND == TRACE_KIND_FAST {
            self.fast_trace_object(queue, object)
        } else {
            unreachable!()
        }
    }

    #[inline(always)]
    fn post_scan_object(&self, object: ObjectReference) {
        if super::MARK_LINE_AT_SCAN_TIME && !super::BLOCK_ONLY {
            debug_assert!(self.in_space(object));
            self.mark_lines(object);
        }
    }

    #[inline(always)]
    fn may_move_objects<const KIND: TraceKind>() -> bool {
        if KIND == TRACE_KIND_DEFRAG {
            true
        } else if KIND == TRACE_KIND_FAST {
            false
        } else {
            unreachable!()
        }
    }
}

impl<VM: VMBinding> ImmixSpace<VM> {
    const UNMARKED_STATE: u8 = 0;
    const MARKED_STATE: u8 = 1;

    /// Get side metadata specs
    fn side_metadata_specs() -> Vec<SideMetadataSpec> {
        metadata::extract_side_metadata(&if super::BLOCK_ONLY {
            vec![
                MetadataSpec::OnSide(Block::DEFRAG_STATE_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                MetadataSpec::OnSide(ChunkMap::ALLOC_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                #[cfg(feature = "object_pinning")]
                *VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC,
            ]
        } else {
            vec![
                MetadataSpec::OnSide(Line::MARK_TABLE),
                MetadataSpec::OnSide(Block::DEFRAG_STATE_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                MetadataSpec::OnSide(ChunkMap::ALLOC_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                #[cfg(feature = "object_pinning")]
                *VM::VMObjectModel::LOCAL_PINNING_BIT_SPEC,
            ]
        })
    }

    pub fn new(
        name: &'static str,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
        scheduler: Arc<GCWorkScheduler<VM>>,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
    ) -> Self {
        #[cfg(feature = "immix_no_defrag")]
        info!(
            "Creating non-moving ImmixSpace: {}. Block size: 2^{}",
            name,
            Block::LOG_BYTES
        );

        super::validate_features();
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                zeroed: true,
                vmrequest: VMRequest::discontiguous(),
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: Self::side_metadata_specs(),
                },
                needs_log_bit: false,
            },
            vm_map,
            mmapper,
            heap,
        );
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
            chunk_map: ChunkMap::new(),
            line_mark_state: AtomicU8::new(Line::RESET_MARK_STATE),
            line_unavail_state: AtomicU8::new(Line::RESET_MARK_STATE),
            reusable_blocks: ReusableBlockPool::new(scheduler.num_workers()),
            defrag: Defrag::default(),
            mark_state: Self::UNMARKED_STATE,
            scheduler,
        }
    }

    /// Flush the thread-local queues in BlockPageResource
    pub fn flush_page_resource(&self) {
        self.reusable_blocks.flush_all();
        #[cfg(target_pointer_width = "64")]
        self.pr.flush_all()
    }

    /// Get the number of defrag headroom pages.
    pub fn defrag_headroom_pages(&self) -> usize {
        self.defrag.defrag_headroom_pages(self)
    }

    /// Check if current GC is a defrag GC.
    #[inline(always)]
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
            emergency_collection,
            collect_whole_heap,
            collection_attempts,
            user_triggered_collection,
            self.reusable_blocks.len() == 0,
            full_heap_system_gc,
        );
        self.defrag.in_defrag()
    }

    /// Get work packet scheduler
    fn scheduler(&self) -> &GCWorkScheduler<VM> {
        &self.scheduler
    }

    pub fn prepare(&mut self, major_gc: bool) {
        if major_gc {
            // Update mark_state
            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_on_side() {
                self.mark_state = Self::MARKED_STATE;
            } else {
                // For header metadata, we use cyclic mark bits.
                unimplemented!("cyclic mark bits is not supported at the moment");
            }
        }

        // Prepare defrag info
        if super::DEFRAG {
            self.defrag.prepare(self);
        }
        // Prepare each block for GC
        let threshold = self.defrag.defrag_spill_threshold.load(Ordering::Acquire);
        // # Safety: ImmixSpace reference is always valid within this collection cycle.
        let space = unsafe { &*(self as *const Self) };
        let work_packets = self.chunk_map.generate_tasks(|chunk| {
            Box::new(PrepareBlockState {
                space,
                chunk,
                defrag_threshold: if space.in_defrag() {
                    Some(threshold)
                } else {
                    None
                },
            })
        });
        self.scheduler().work_buckets[WorkBucketStage::Prepare].bulk_add(work_packets);
        // Update line mark state
        if !super::BLOCK_ONLY {
            self.line_mark_state.fetch_add(1, Ordering::AcqRel);
            if self.line_mark_state.load(Ordering::Acquire) > Line::MAX_MARK_STATE {
                self.line_mark_state
                    .store(Line::RESET_MARK_STATE, Ordering::Release);
            }
        }
    }

    /// Release for the immix space. This is called when a GC finished.
    /// Return whether this GC was a defrag GC, as a plan may want to know this.
    pub fn release(&mut self, major_gc: bool) -> bool {
        let did_defrag = self.defrag.in_defrag();
        if major_gc {
            // Update line_unavail_state for hole searching afte this GC.
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
        // # Safety: ImmixSpace reference is always valid within this collection cycle.
        let space = unsafe { &*(self as *const Self) };
        let work_packets = self.chunk_map.generate_sweep_tasks(space);
        self.scheduler().work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
        if super::DEFRAG {
            self.defrag.release(self);
        }
        did_defrag
    }

    /// Release a block.
    pub fn release_block(&self, block: Block) {
        block.deinit();
        self.pr.release_block(block);
    }

    /// Allocate a clean block.
    pub fn get_clean_block(&self, tls: VMThread, copy: bool) -> Option<Block> {
        let block_address = self.acquire(tls, Block::PAGES);
        if block_address.is_zero() {
            return None;
        }
        self.defrag.notify_new_clean_block(copy);
        let block = Block::from_aligned_address(block_address);
        block.init(copy);
        self.chunk_map.set(block.chunk(), ChunkState::Allocated);
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
                block.init(copy);
                return Some(block);
            } else {
                return None;
            }
        }
    }

    /// Trace and mark objects without evacuation.
    #[inline(always)]
    pub fn fast_trace_object(
        &self,
        trace: &mut impl ObjectQueue,
        object: ObjectReference,
    ) -> ObjectReference {
        self.trace_object_without_moving(trace, object)
    }

    /// Trace and mark objects. If the current object is in defrag block, then do evacuation as well.
    #[inline(always)]
    pub fn trace_object(
        &self,
        trace: &mut impl ObjectQueue,
        object: ObjectReference,
        semantics: CopySemantics,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        #[cfg(feature = "global_alloc_bit")]
        debug_assert!(
            crate::util::alloc_bit::is_alloced::<VM>(object),
            "{:x}: alloc bit not set",
            object
        );
        if Block::containing::<VM>(object).is_defrag_source() {
            debug_assert!(self.in_defrag());
            self.trace_object_with_opportunistic_copy(trace, object, semantics, worker)
        } else {
            self.trace_object_without_moving(trace, object)
        }
    }

    /// Trace and mark objects without evacuation.
    #[inline(always)]
    pub fn trace_object_without_moving(
        &self,
        queue: &mut impl ObjectQueue,
        object: ObjectReference,
    ) -> ObjectReference {
        if self.attempt_mark(object, self.mark_state) {
            // Mark block and lines
            if !super::BLOCK_ONLY {
                if !super::MARK_LINE_AT_SCAN_TIME {
                    self.mark_lines(object);
                }
            } else {
                Block::containing::<VM>(object).set_state(BlockState::Marked);
            }
            // Visit node
            queue.enqueue(object);
        }
        object
    }

    /// Trace object and do evacuation if required.
    #[allow(clippy::assertions_on_constants)]
    #[inline(always)]
    pub fn trace_object_with_opportunistic_copy(
        &self,
        queue: &mut impl ObjectQueue,
        object: ObjectReference,
        semantics: CopySemantics,
        worker: &mut GCWorker<VM>,
    ) -> ObjectReference {
        let copy_context = worker.get_copy_context_mut();
        debug_assert!(!super::BLOCK_ONLY);
        let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
            // We lost the forwarding race as some other thread has set the forwarding word; wait
            // until the object has been forwarded by the winner. Note that the object may not
            // necessarily get forwarded since Immix opportunistically moves objects.
            #[allow(clippy::let_and_return)]
            let new_object =
                ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status);
            #[cfg(debug_assertions)]
            {
                if new_object == object {
                    debug_assert!(
                        self.is_marked(object, self.mark_state) || self.defrag.space_exhausted() || self.is_pinned(object),
                        "Forwarded object is the same as original object {} even though it should have been copied",
                        object,
                    );
                } else {
                    // new_object != object
                    debug_assert!(
                        !Block::containing::<VM>(new_object).is_defrag_source(),
                        "Block {:?} containing forwarded object {} should not be a defragmentation source",
                        Block::containing::<VM>(new_object),
                        new_object,
                    );
                }
            }
            new_object
        } else if self.is_marked(object, self.mark_state) {
            // We won the forwarding race but the object is already marked so we clear the
            // forwarding status and return the unmoved object
            ForwardingWord::clear_forwarding_bits::<VM>(object);
            object
        } else {
            // We won the forwarding race; actually forward and copy the object if it is not pinned
            // and we have sufficient space in our copy allocator
            let new_object = if self.is_pinned(object) || self.defrag.space_exhausted() {
                self.attempt_mark(object, self.mark_state);
                ForwardingWord::clear_forwarding_bits::<VM>(object);
                Block::containing::<VM>(object).set_state(BlockState::Marked);
                object
            } else {
                #[cfg(feature = "global_alloc_bit")]
                crate::util::alloc_bit::unset_alloc_bit::<VM>(object);
                ForwardingWord::forward_object::<VM>(object, semantics, copy_context)
            };
            debug_assert_eq!(
                Block::containing::<VM>(new_object).get_state(),
                BlockState::Marked
            );
            queue.enqueue(new_object);
            debug_assert!(new_object.is_live());
            new_object
        }
    }

    /// Mark all the lines that the given object spans.
    #[allow(clippy::assertions_on_constants)]
    #[inline]
    pub fn mark_lines(&self, object: ObjectReference) {
        debug_assert!(!super::BLOCK_ONLY);
        Line::mark_lines_for_object::<VM>(object, self.line_mark_state.load(Ordering::Acquire));
    }

    /// Atomically mark an object.
    #[inline(always)]
    fn attempt_mark(&self, object: ObjectReference, mark_state: u8) -> bool {
        loop {
            let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
                object,
                None,
                Ordering::SeqCst,
            );
            if old_value == mark_state {
                return false;
            }

            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .compare_exchange_metadata::<VM, u8>(
                    object,
                    old_value,
                    mark_state,
                    None,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break;
            }
        }
        true
    }

    /// Check if an object is marked.
    #[inline(always)]
    fn is_marked(&self, object: ObjectReference, mark_state: u8) -> bool {
        let old_value = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.load_atomic::<VM, u8>(
            object,
            None,
            Ordering::SeqCst,
        );
        old_value == mark_state
    }

    /// Check if an object is pinned.
    #[inline(always)]
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
    pub fn get_next_available_lines(&self, search_start: Line) -> Option<(Line, Line)> {
        debug_assert!(!super::BLOCK_ONLY);
        let unavail_state = self.line_unavail_state.load(Ordering::Acquire);
        let current_state = self.line_mark_state.load(Ordering::Acquire);
        let block = search_start.block();
        let mark_data = block.line_mark_table();
        let start_cursor = search_start.get_index_within_block();
        let mut cursor = start_cursor;
        // Find start
        while cursor < mark_data.len() {
            let mark = mark_data.get(cursor);
            if mark != unavail_state && mark != current_state {
                break;
            }
            cursor += 1;
        }
        if cursor == mark_data.len() {
            return None;
        }
        let start = search_start.next_nth(cursor - start_cursor);
        // Find limit
        while cursor < mark_data.len() {
            let mark = mark_data.get(cursor);
            if mark == unavail_state || mark == current_state {
                break;
            }
            cursor += 1;
        }
        let end = search_start.next_nth(cursor - start_cursor);
        debug_assert!(RegionIterator::<Line>::new(start, end)
            .all(|line| !line.is_marked(unavail_state) && !line.is_marked(current_state)));
        Some((start, end))
    }

    pub fn is_last_gc_exhaustive(did_defrag_for_last_gc: bool) -> bool {
        if super::DEFRAG {
            did_defrag_for_last_gc
        } else {
            // If defrag is disabled, every GC is exhaustive.
            true
        }
    }
}

/// A work packet to prepare each block for GC.
/// Performs the action on a range of chunks.
pub struct PrepareBlockState<VM: VMBinding> {
    pub space: &'static ImmixSpace<VM>,
    pub chunk: Chunk,
    pub defrag_threshold: Option<usize>,
}

impl<VM: VMBinding> PrepareBlockState<VM> {
    /// Clear object mark table
    #[inline(always)]
    fn reset_object_mark(chunk: Chunk) {
        if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
            side.bzero_metadata(chunk.start(), Chunk::BYTES);
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for PrepareBlockState<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let defrag_threshold = self.defrag_threshold.unwrap_or(0);
        // Clear object mark table for this chunk
        Self::reset_object_mark(self.chunk);
        // Iterate over all blocks in this chunk
        for block in self.chunk.blocks() {
            let state = block.get_state();
            // Skip unallocated blocks.
            if state == BlockState::Unallocated {
                continue;
            }
            // Check if this block needs to be defragmented.
            if super::DEFRAG && defrag_threshold != 0 && block.get_holes() > defrag_threshold {
                block.set_as_defrag_source(true);
            } else {
                block.set_as_defrag_source(false);
            }
            // Clear block mark data.
            block.set_state(BlockState::Unmarked);
            debug_assert!(!block.get_state().is_reusable());
            debug_assert_ne!(block.get_state(), BlockState::Marked);
        }
    }
}

use crate::plan::{Plan, VectorObjectQueue};
use crate::policy::copy_context::PolicyCopyContext;
use crate::util::alloc::Allocator;
use crate::util::alloc::ImmixAllocator;

/// Immix copy allocator
pub struct ImmixCopyContext<VM: VMBinding> {
    copy_allocator: ImmixAllocator<VM>,
    defrag_allocator: ImmixAllocator<VM>,
}

impl<VM: VMBinding> PolicyCopyContext for ImmixCopyContext<VM> {
    type VM = VM;

    fn prepare(&mut self) {
        self.copy_allocator.reset();
        self.defrag_allocator.reset();
    }
    fn release(&mut self) {
        self.copy_allocator.reset();
        self.defrag_allocator.reset();
    }
    #[inline(always)]
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
    ) -> Address {
        if self.get_space().in_defrag() {
            self.defrag_allocator.alloc(bytes, align, offset)
        } else {
            self.copy_allocator.alloc(bytes, align, offset)
        }
    }
    #[inline(always)]
    fn post_copy(&mut self, obj: ObjectReference, _bytes: usize) {
        // Mark the object
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.store_atomic::<VM, u8>(
            obj,
            self.get_space().mark_state,
            None,
            Ordering::SeqCst,
        );
        // Mark the line
        if !super::MARK_LINE_AT_SCAN_TIME {
            self.get_space().mark_lines(obj);
        }
    }
}

impl<VM: VMBinding> ImmixCopyContext<VM> {
    pub fn new(
        tls: VMWorkerThread,
        plan: &'static dyn Plan<VM = VM>,
        space: &'static ImmixSpace<VM>,
    ) -> Self {
        ImmixCopyContext {
            copy_allocator: ImmixAllocator::new(tls.0, Some(space), plan, false),
            defrag_allocator: ImmixAllocator::new(tls.0, Some(space), plan, true),
        }
    }

    #[inline(always)]
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
