use super::line::*;
use super::{
    block::*,
    chunk::{Chunk, ChunkMap, ChunkState},
    defrag::Defrag,
};
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::heap::PageResource;
use crate::util::heap::VMRequest;
use crate::util::metadata::side_metadata::{self, *};
use crate::util::metadata::{self, compare_exchange_metadata, load_metadata, MetadataSpec};
use crate::util::object_forwarding as ForwardingWord;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::{
    plan::TransitiveClosure,
    scheduler::{gc_work::ProcessEdgesWork, GCWork, GCWorker, MMTkScheduler, WorkBucketStage},
    util::{
        heap::FreeListPageResource,
        opaque_pointer::{VMThread, VMWorkerThread},
    },
    AllocationSemantics, CopyContext, MMTK,
};
use atomic::Ordering;
use std::{
    iter::Step,
    mem,
    ops::Range,
    sync::{
        atomic::{AtomicBool, AtomicU8},
        Arc, Weak,
    },
};

pub struct ImmixSpace<VM: VMBinding> {
    common: CommonSpace<VM>,
    pr: FreeListPageResource<VM>,
    /// Allocation status for all chunks in immix space
    pub chunk_map: ChunkMap,
    /// Current line mark state
    pub line_mark_state: AtomicU8,
    /// Line mark state in previous GC
    line_unavail_state: AtomicU8,
    /// Is in a GC?
    in_collection: AtomicBool,
    /// A list of all reusable blocks
    pub reusable_blocks: BlockList,
    /// Defrag utilities
    pub(super) defrag: Defrag,
    /// Object mark state
    mark_state: usize,
    /// Work packet scheduler
    scheduler: Weak<MMTkScheduler<VM>>,
}

unsafe impl<VM: VMBinding> Sync for ImmixSpace<VM> {}

impl<VM: VMBinding> SFT for ImmixSpace<VM> {
    fn name(&self) -> &str {
        self.get_name()
    }
    fn is_live(&self, object: ObjectReference) -> bool {
        ForwardingWord::is_forwarded::<VM>(object)
    }
    fn is_movable(&self) -> bool {
        true
    }
    #[cfg(feature = "sanity")]
    fn is_sane(&self) -> bool {
        true
    }
    fn initialize_object_metadata(&self, _object: ObjectReference, _alloc: bool) {}
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
    fn init(&mut self, _vm_map: &'static VMMap) {
        super::validate_features();
        self.common().init(self.as_space());
    }
    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immixspace only releases pages enmasse")
    }
}

impl<VM: VMBinding> ImmixSpace<VM> {
    const UNMARKED_STATE: usize = 0;
    const MARKED_STATE: usize = 1;

    /// Get side metadata specs
    #[allow(clippy::assertions_on_constants)]
    fn side_metadata_specs() -> Vec<SideMetadataSpec> {
        metadata::extract_side_metadata(&if super::BLOCK_ONLY {
            vec![
                MetadataSpec::OnSide(Block::DEFRAG_STATE_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                MetadataSpec::OnSide(ChunkMap::ALLOC_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            ]
        } else {
            vec![
                MetadataSpec::OnSide(Line::MARK_TABLE),
                MetadataSpec::OnSide(Block::DEFRAG_STATE_TABLE),
                MetadataSpec::OnSide(Block::MARK_TABLE),
                MetadataSpec::OnSide(ChunkMap::ALLOC_TABLE),
                *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            ]
        })
    }

    pub fn new(
        name: &'static str,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
        scheduler: Arc<MMTkScheduler<VM>>,
        global_side_metadata_specs: Vec<SideMetadataSpec>,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                zeroed: true,
                vmrequest: VMRequest::discontiguous(),
                side_metadata_specs: SideMetadataContext {
                    global: global_side_metadata_specs,
                    local: Self::side_metadata_specs().to_vec(),
                },
            },
            vm_map,
            mmapper,
            heap,
        );
        ImmixSpace {
            pr: if common.vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common,
            chunk_map: ChunkMap::new(),
            line_mark_state: AtomicU8::new(Line::RESET_MARK_STATE),
            line_unavail_state: AtomicU8::new(Line::RESET_MARK_STATE),
            in_collection: AtomicBool::new(false),
            reusable_blocks: BlockList::default(),
            defrag: Defrag::new(),
            mark_state: Self::UNMARKED_STATE,
            scheduler: Arc::downgrade(&scheduler),
        }
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
    pub fn decide_whether_to_defrag(&self, emergency_collection: bool, collection_attempts: usize) {
        self.defrag.decide_whether_to_defrag(
            emergency_collection,
            collection_attempts,
            self.reusable_blocks.len() == 0,
        )
    }

    const AVAILABLE_LOCAL_BITS: usize = 7;
    const MARK_BASE: usize = 4;
    const MARK_INCREMENT: u8 = 1 << Self::MARK_BASE;
    const MAX_MARKCOUNT_BITS: usize = Self::AVAILABLE_LOCAL_BITS - Self::MARK_BASE;
    const MARK_MASK: u8 = ((1 << Self::MAX_MARKCOUNT_BITS) - 1) << Self::MARK_BASE;
    const MARK_BASE_VALUE: u8 = Self::MARK_INCREMENT;

    /// Update mark state
    #[allow(clippy::assertions_on_constants)]
    fn delta_mark_state(state: u8) -> u8 {
        debug_assert!(!VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_on_side());
        let mut rtn = state;
        loop {
            rtn = (rtn + Self::MARK_INCREMENT) & Self::MARK_MASK;
            if rtn >= Self::MARK_BASE_VALUE {
                break;
            }
        }
        debug_assert_ne!(rtn, state);
        rtn
    }

    /// Get work packet scheduler
    fn scheduler(&self) -> Arc<MMTkScheduler<VM>> {
        self.scheduler.upgrade().unwrap()
    }

    pub fn prepare(&mut self) {
        // Update mark_state
        if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.is_on_side() {
            // For side metadata, we always use `1` as marked state.
            // Object mark table will be cleared by `PrepareBlockState` before each GC.
            //
            // Note: It is incorrect to flip matk bit between 0 and 1 and remove
            // the mark-table zeroing step. Because openjdk does not call post_alloc to set up
            // object initial metadata.
            self.mark_state = Self::MARKED_STATE;
        } else {
            // For header metadata, we use cyclic mark bits.
            self.mark_state = Self::delta_mark_state(self.mark_state as u8) as usize;
        }
        // Prepare defrag info
        if super::DEFRAG {
            self.defrag.prepare(self);
        }
        // Prepare each block for GC
        let threshold = self.defrag.defrag_spill_threshold.load(Ordering::Acquire);
        // # Safety: ImmixSpace reference is always valid within this collection cycle.
        let space = unsafe { &*(self as *const Self) };
        let work_packets =
            self.chunk_map
                .generate_tasks(self.scheduler().num_workers(), |chunks| {
                    box PrepareBlockState {
                        space,
                        chunks,
                        defrag_threshold: if space.in_defrag() {
                            Some(threshold)
                        } else {
                            None
                        },
                    }
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
        self.in_collection.store(true, Ordering::Release);
    }

    pub fn release(&mut self) {
        // Update line_unavail_state for hole searching afte this GC.
        if !super::BLOCK_ONLY {
            self.line_unavail_state.store(
                self.line_mark_state.load(Ordering::Acquire),
                Ordering::Release,
            );
        }
        // Clear reusable blocks list
        if !super::BLOCK_ONLY {
            self.reusable_blocks.reset();
        }
        // Sweep chunks and blocks
        // # Safety: ImmixSpace reference is always valid within this collection cycle.
        let space = unsafe { &*(self as *const Self) };
        let work_packets = self
            .chunk_map
            .generate_sweep_tasks(space, &self.scheduler());
        self.scheduler().work_buckets[WorkBucketStage::Release].bulk_add(work_packets);
        // Update states
        self.in_collection.store(false, Ordering::Release);
        if super::DEFRAG {
            self.defrag.release(self)
        }
    }

    /// Release a block.
    pub fn release_block(&self, block: Block) {
        block.deinit();
        self.pr.release_pages(block.start());
    }

    /// Allocate a clean block.
    pub fn get_clean_block(&self, tls: VMThread, copy: bool) -> Option<Block> {
        let block_address = self.acquire(tls, Block::PAGES);
        if block_address.is_zero() {
            return None;
        }
        self.defrag.notify_new_clean_block(copy);
        let block = Block::from(block_address);
        block.init(copy);
        self.chunk_map.set(block.chunk(), ChunkState::Allocated);
        Some(block)
    }

    /// Pop a reusable block from the reusable block list.
    pub fn get_reusable_block(&self, copy: bool) -> Option<Block> {
        if super::BLOCK_ONLY {
            return None;
        }
        let result = self.reusable_blocks.pop();
        if let Some(block) = result {
            // println!("Reuse {:?}", block);
            block.init(copy);
        }
        result
    }

    /// Trace and mark objects without evacuation.
    #[inline(always)]
    pub fn fast_trace_object(
        &self,
        trace: &mut impl TransitiveClosure,
        object: ObjectReference,
    ) -> ObjectReference {
        self.trace_object_without_moving(trace, object)
    }

    /// Trace and mark objects. If the current object is in defrag block, then do evacuation as well.
    #[inline(always)]
    pub fn trace_object(
        &self,
        trace: &mut impl TransitiveClosure,
        object: ObjectReference,
        semantics: AllocationSemantics,
        copy_context: &mut impl CopyContext,
    ) -> ObjectReference {
        if Block::containing::<VM>(object).is_defrag_source() {
            self.trace_object_with_opportunistic_copy(trace, object, semantics, copy_context)
        } else {
            self.trace_object_without_moving(trace, object)
        }
    }

    /// Trace and mark objects without evacuation.
    #[inline(always)]
    pub fn trace_object_without_moving(
        &self,
        trace: &mut impl TransitiveClosure,
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
            trace.process_node(object);
        }
        object
    }

    /// Trace object and do evacuation if required.
    #[allow(clippy::assertions_on_constants)]
    #[inline(always)]
    pub fn trace_object_with_opportunistic_copy(
        &self,
        trace: &mut impl TransitiveClosure,
        object: ObjectReference,
        semantics: AllocationSemantics,
        copy_context: &mut impl CopyContext,
    ) -> ObjectReference {
        debug_assert!(!super::BLOCK_ONLY);
        let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
            ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status)
        } else if self.is_marked(object, self.mark_state) {
            ForwardingWord::clear_forwarding_bits::<VM>(object);
            object
        } else {
            let new_object = if Self::is_pinned(object) || self.defrag.space_exhausted() {
                self.attempt_mark(object, self.mark_state);
                ForwardingWord::clear_forwarding_bits::<VM>(object);
                Block::containing::<VM>(object).set_state(BlockState::Marked);
                object
            } else {
                ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context)
            };
            if !super::MARK_LINE_AT_SCAN_TIME {
                self.mark_lines(new_object);
            }
            debug_assert_eq!(
                Block::containing::<VM>(new_object).get_state(),
                BlockState::Marked
            );
            trace.process_node(new_object);
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
    fn attempt_mark(&self, object: ObjectReference, mark_state: usize) -> bool {
        loop {
            let old_value = load_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                None,
                Some(Ordering::SeqCst),
            );
            if old_value == mark_state {
                return false;
            }

            if compare_exchange_metadata::<VM>(
                &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
                object,
                old_value,
                mark_state,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                break;
            }
        }
        true
    }

    /// Check if an object is marked.
    #[inline(always)]
    fn is_marked(&self, object: ObjectReference, mark_state: usize) -> bool {
        let old_value = load_metadata::<VM>(
            &VM::VMObjectModel::LOCAL_MARK_BIT_SPEC,
            object,
            None,
            Some(Ordering::SeqCst),
        );
        old_value == mark_state
    }

    /// Check if an object is pinned.
    #[inline(always)]
    fn is_pinned(_object: ObjectReference) -> bool {
        // TODO(wenyuzhao): Object pinning not supported yet.
        false
    }

    /// Hole searching.
    ///
    /// Linearly scan lines in a block to search for the next
    /// hole, starting from the given line.
    ///
    /// Returns None if the search could not find any more holes.
    #[allow(clippy::assertions_on_constants)]
    pub fn get_next_available_lines(&self, search_start: Line) -> Option<Range<Line>> {
        debug_assert!(!super::BLOCK_ONLY);
        let unavail_state = self.line_unavail_state.load(Ordering::Acquire);
        let current_state = self.line_mark_state.load(Ordering::Acquire);
        let block = search_start.block();
        let mark_data = block.line_mark_table();
        let start_cursor = search_start.get_index_within_block();
        let mut cursor = start_cursor;
        // Find start
        while cursor < mark_data.len() {
            let mark = mark_data[cursor].load(Ordering::Relaxed);
            if mark != unavail_state && mark != current_state {
                break;
            }
            cursor += 1;
        }
        if cursor == mark_data.len() {
            return None;
        }
        let start = Line::forward(search_start, cursor - start_cursor);
        // Find limit
        while cursor < mark_data.len() {
            let mark = mark_data[cursor].load(Ordering::Relaxed);
            if mark == unavail_state || mark == current_state {
                break;
            }
            cursor += 1;
        }
        let end = Line::forward(search_start, cursor - start_cursor);
        debug_assert!((start..end)
            .all(|line| !line.is_marked(unavail_state) && !line.is_marked(current_state)));
        Some(start..end)
    }
}

/// A work packet to prepare each block for GC.
/// Performs the action on a range of chunks.
pub struct PrepareBlockState<VM: VMBinding> {
    pub space: &'static ImmixSpace<VM>,
    pub chunks: Range<Chunk>,
    pub defrag_threshold: Option<usize>,
}

impl<VM: VMBinding> PrepareBlockState<VM> {
    /// Clear object mark table
    #[inline(always)]
    fn reset_object_mark(chunk: Chunk) {
        if let MetadataSpec::OnSide(side) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
            side_metadata::bzero_metadata(&side, chunk.start(), Chunk::BYTES);
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for PrepareBlockState<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let defrag_threshold = self.defrag_threshold.unwrap_or(0);
        for chunk in self
            .chunks
            .clone()
            .filter(|c| self.space.chunk_map.get(*c) == ChunkState::Allocated)
        {
            // Clear object mark table for this chunk
            Self::reset_object_mark(chunk);
            // Iterate over all blocks in this chunk
            for block in chunk.blocks() {
                let state = block.get_state();
                // Skip unallocated blocks.
                if state == BlockState::Unallocated {
                    continue;
                }
                // Check if this block needs to be defragmented.
                if super::DEFRAG
                    && defrag_threshold != 0
                    && !state.is_reusable()
                    && block.get_holes() > defrag_threshold
                {
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
}

/// A transitive closure visitor to collect all the edges of an object.
pub struct ObjectsClosure<'a, E: ProcessEdgesWork>(
    &'static MMTK<E::VM>,
    Vec<Address>,
    &'a mut GCWorker<E::VM>,
);

impl<'a, E: ProcessEdgesWork> TransitiveClosure for ObjectsClosure<'a, E> {
    #[inline(always)]
    fn process_edge(&mut self, slot: Address) {
        if self.1.is_empty() {
            self.1.reserve(E::CAPACITY);
        }
        self.1.push(slot);
        if self.1.len() >= E::CAPACITY {
            let mut new_edges = Vec::new();
            mem::swap(&mut new_edges, &mut self.1);
            self.2
                .add_work(WorkBucketStage::Closure, E::new(new_edges, false, self.0));
        }
    }
    fn process_node(&mut self, _object: ObjectReference) {
        unreachable!()
    }
}

impl<'a, E: ProcessEdgesWork> Drop for ObjectsClosure<'a, E> {
    #[inline(always)]
    fn drop(&mut self) {
        let mut new_edges = Vec::new();
        mem::swap(&mut new_edges, &mut self.1);
        self.2
            .add_work(WorkBucketStage::Closure, E::new(new_edges, false, self.0));
    }
}

/// A work packet to scan the fields of each objects and mark lines.
pub struct ScanObjectsAndMarkLines<Edges: ProcessEdgesWork> {
    buffer: Vec<ObjectReference>,
    #[allow(unused)]
    concurrent: bool,
    immix_space: &'static ImmixSpace<Edges::VM>,
}

impl<Edges: ProcessEdgesWork> ScanObjectsAndMarkLines<Edges> {
    pub fn new(
        buffer: Vec<ObjectReference>,
        concurrent: bool,
        immix_space: &'static ImmixSpace<Edges::VM>,
    ) -> Self {
        Self {
            buffer,
            concurrent,
            immix_space,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ScanObjectsAndMarkLines<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        trace!("ScanObjectsAndMarkLines");
        let mut closure = ObjectsClosure::<E>(mmtk, vec![], worker);
        for object in &self.buffer {
            <E::VM as VMBinding>::VMScanning::scan_object(
                &mut closure,
                *object,
                VMWorkerThread(VMThread::UNINITIALIZED),
            );
            if super::MARK_LINE_AT_SCAN_TIME
                && !super::BLOCK_ONLY
                && self.immix_space.in_space(*object)
            {
                self.immix_space.mark_lines(*object);
            }
        }
    }
}
