use atomic::Ordering;
use crate::{AllocationSemantics, CopyContext, MMTK, plan::TransitiveClosure, scheduler::{GCWork, GCWorkBucket, GCWorker, WorkBucketStage, gc_work::ProcessEdgesWork}, util::{OpaquePointer, constants::{LOG_BYTES_IN_WORD}, gc_byte, heap::FreeListPageResource}};
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::forwarding_word as ForwardingWord;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::heap::PageResource;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::util::side_metadata::{self, *};
use std::{cell::UnsafeCell, iter::Step, mem, ops::Range, sync::atomic::{AtomicBool, AtomicU8}};
use super::{block::*, chunk::{Chunk, ChunkMap, ChunkState}, defrag::Defrag};
use super::line::*;



pub struct ImmixSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: FreeListPageResource<VM>,
    pub chunk_map: ChunkMap,
    pub line_mark_state: AtomicU8,
    line_unavail_state: AtomicU8,
    in_collection: AtomicBool,
    pub reusable_blocks: BlockList,
    pub(super) defrag: Defrag,
    mark_state: AtomicU8,
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
    fn initialize_header(&self, object: ObjectReference, _alloc: bool) {
        debug_assert!(Self::HEADER_MARK_BITS);
        let old_value = gc_byte::read_gc_byte::<VM>(object);
        let new_value = (old_value & Self::GC_MARK_BIT_MASK) | self.mark_state.load(Ordering::Acquire);
        gc_byte::write_gc_byte::<VM>(object, new_value);
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
        unsafe { &*self.common.get() }
    }
    unsafe fn unsafe_common_mut(&self) -> &mut CommonSpace<VM> {
        &mut *self.common.get()
    }
    fn init(&mut self, _vm_map: &'static VMMap) {
        super::validate_features();
        // Borrow-checker fighting so that we can have a cyclic reference
        let me = unsafe { &*(self as *const Self) };
        self.pr.bind_space(me);
        self.common().init(self.as_space());
    }
    fn release_multiple_pages(&mut self, _start: Address) {
        panic!("immixspace only releases pages enmasse")
    }

    fn local_side_metadata_per_chunk(&self) -> usize {
        Self::LOCAL_SIDE_METADATA_PER_CHUNK
    }
}

impl<VM: VMBinding> ImmixSpace<VM> {
    pub const LOCAL_SIDE_METADATA_PER_CHUNK: usize = {
        if Self::HEADER_MARK_BITS {
            Block::MARK_TABLE.accumulated_size()
        } else {
            Self::OBJECT_MARK_TABLE.accumulated_size()
        }
    };

    pub fn new(
        name: &'static str,
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        heap: &mut HeapMeta,
    ) -> Self {
        let common = CommonSpace::new(
            SpaceOptions {
                name,
                movable: true,
                immortal: false,
                zeroed: true,
                vmrequest: VMRequest::discontiguous(),
            },
            vm_map,
            mmapper,
            heap,
        );
        #[cfg(target_pointer_width = "64")]
        let start = common.start;
        #[cfg(target_pointer_width = "32")]
        let start = crate::util::heap::layout::vm_layout_constants::HEAP_START;
        ImmixSpace {
            pr: if common.vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common: UnsafeCell::new(common),
            chunk_map: ChunkMap::new(start),
            line_mark_state: AtomicU8::new(Line::RESET_MARK_STATE),
            line_unavail_state: AtomicU8::new(Line::RESET_MARK_STATE),
            in_collection: AtomicBool::new(false),
            reusable_blocks: BlockList::new(),
            defrag: Defrag::new(),
            mark_state: AtomicU8::new(0),
        }
    }

    pub fn defrag_headroom_pages(&self) -> usize {
        self.defrag.defrag_headroom_pages(self)
    }

    #[inline(always)]
    pub fn in_defrag(&self) -> bool {
        self.defrag.in_defrag()
    }

    pub fn initialize_defrag(&self, mmtk: &MMTK<VM>) {
        self.defrag.prepare_histograms(mmtk);
    }

    pub fn decide_whether_to_defrag(&self, emergency_collection: bool, collection_attempts: usize) {
        self.defrag.decide_whether_to_defrag(emergency_collection, collection_attempts, self.reusable_blocks.len() == 0)
    }

    pub fn prepare(&'static self, mmtk: &'static MMTK<VM>) {
        if Self::HEADER_MARK_BITS {
            self.mark_state.store(Self::GC_MARK_BIT_MASK - self.mark_state.load(Ordering::Acquire), Ordering::Release);
        }
        if !super::BLOCK_ONLY {
            self.defrag.prepare(self);
        }

        let threshold = self.defrag.defrag_spill_threshold.load(Ordering::Acquire);
        let work_packets = self.chunk_map.generate_tasks(mmtk.scheduler.num_workers(), |chunks| {
            box PrepareBlockState {
                space: self,
                chunks,
                defrag_threshold: if self.in_defrag() { Some(threshold) } else { None },
            }
        });
        mmtk.scheduler.work_buckets[WorkBucketStage::Prepare].bulk_add(GCWorkBucket::<VM>::DEFAULT_PRIORITY, work_packets);

        if !super::BLOCK_ONLY {
            self.line_mark_state.fetch_add(1, Ordering::AcqRel);
            if self.line_mark_state.load(Ordering::Acquire) > Line::MAX_MARK_STATE {
                self.line_mark_state.store(Line::RESET_MARK_STATE, Ordering::Release);
            }
        }
        self.in_collection.store(true, Ordering::Release);
    }

    pub fn release(&'static self, mmtk: &'static MMTK<VM>) {
        if !super::BLOCK_ONLY {
            self.line_unavail_state.store(self.line_mark_state.load(Ordering::Acquire), Ordering::Release);
        }

        if !super::BLOCK_ONLY {
            self.reusable_blocks.reset();
        }

        let work_packets = self.chunk_map.generate_sweep_tasks(self, mmtk);
        mmtk.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(GCWorkBucket::<VM>::DEFAULT_PRIORITY, work_packets);

        self.in_collection.store(false, Ordering::Release);
        if !super::BLOCK_ONLY {
            self.defrag.release(self)
        }
    }

    pub fn release_block(&self, block: Block) {
        block.deinit();
        self.pr.release_pages(block.start());
    }

    pub fn get_clean_block(&self, tls: OpaquePointer, copy: bool) -> Option<Block> {
        let block_address = self.acquire(tls, Block::PAGES);
        if block_address.is_zero() { return None }
        self.defrag.notify_new_clean_block(copy);
        let block = Block::from(block_address);
        block.init(copy);
        self.chunk_map.set(block.chunk(), ChunkState::Allocated);
        Some(block)
    }

    pub fn get_reusable_block(&self, copy: bool) -> Option<Block> {
        if super::BLOCK_ONLY { return None }
        let result = self.reusable_blocks.pop();
        if let Some(block) = result {
            // println!("Reuse {:?}", block);
            block.init(copy);
        }
        result
    }

    #[inline(always)]
    pub fn fast_trace_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference) -> ObjectReference {
        self.trace_object_without_moving(trace, object)
    }

    #[inline(always)]
    pub fn trace_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference, semantics: AllocationSemantics, copy_context: &mut impl CopyContext) -> ObjectReference {
        if Block::containing::<VM>(object).is_defrag_source() {
            self.trace_object_with_opportunistic_copy(trace, object, semantics, copy_context)
        } else {
            self.trace_object_without_moving(trace, object)
        }
    }

    #[inline(always)]
    pub fn trace_object_without_moving(&self, trace: &mut impl TransitiveClosure, object: ObjectReference) -> ObjectReference {
        if self.attempt_mark(object) {
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

    #[inline(always)]
    pub fn trace_object_with_opportunistic_copy(&self, trace: &mut impl TransitiveClosure, object: ObjectReference, semantics: AllocationSemantics, copy_context: &mut impl CopyContext) -> ObjectReference {
        debug_assert!(!super::BLOCK_ONLY);
        let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
            ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status)
        } else {
            if self.is_marked(object) {
                ForwardingWord::clear_forwarding_bits::<VM>(object);
                return object;
            } else {
                let new_object = if Self::is_pinned(object) || self.defrag.space_exhausted() {
                    self.attempt_mark(object);
                    ForwardingWord::clear_forwarding_bits::<VM>(object);
                    Block::containing::<VM>(object).set_state(BlockState::Marked);
                    object
                } else {
                    ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context)
                };
                if !super::MARK_LINE_AT_SCAN_TIME {
                    self.mark_lines(new_object);
                }
                debug_assert_eq!(Block::containing::<VM>(new_object).get_state(), BlockState::Marked);
                trace.process_node(new_object);
                new_object
            }
        }
    }

    /* Line marking */

    #[inline]
    pub fn mark_lines(&self, object: ObjectReference) {
        debug_assert!(!super::BLOCK_ONLY);
        Line::mark_lines_for_object::<VM>(object, self.line_mark_state.load(Ordering::Acquire));
    }

    /* Object Marking */

    const HEADER_MARK_BITS: bool = cfg!(feature = "immix_header_mark_bits");

    const GC_MARK_BIT_MASK: u8 = 1;

    const OBJECT_MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: Block::MARK_TABLE.accumulated_size(),
        log_num_of_bits: 0,
        log_min_obj_size: LOG_BYTES_IN_WORD as usize,
    };

    #[inline(always)]
    fn attempt_mark(&self, object: ObjectReference) -> bool {
        if Self::HEADER_MARK_BITS {
            let mut old_value = gc_byte::read_gc_byte::<VM>(object);
            let mut mark_bit = old_value & Self::GC_MARK_BIT_MASK;
            let value = self.mark_state.load(Ordering::Acquire);
            if mark_bit == value {
                return false;
            }
            while !gc_byte::compare_exchange_gc_byte::<VM>(
                object,
                old_value,
                old_value ^ Self::GC_MARK_BIT_MASK,
            ) {
                old_value = gc_byte::read_gc_byte::<VM>(object);
                mark_bit = (old_value as u8) & Self::GC_MARK_BIT_MASK;
                if mark_bit == value {
                    return false;
                }
            }
            true
        } else {
            side_metadata::compare_exchange_atomic(Self::OBJECT_MARK_TABLE, VM::VMObjectModel::ref_to_address(object), 0, 1)
        }
    }

    #[inline(always)]
    fn is_marked(&self, object: ObjectReference) -> bool {
        if Self::HEADER_MARK_BITS {
            let value = self.mark_state.load(Ordering::Acquire);
            let old_value = gc_byte::read_gc_byte::<VM>(object);
            let mark_bit = old_value & Self::GC_MARK_BIT_MASK;
            mark_bit == value
        } else {
            side_metadata::load_atomic(Self::OBJECT_MARK_TABLE, VM::VMObjectModel::ref_to_address(object)) == 1
        }
    }

    #[inline(always)]
    fn is_pinned(_object: ObjectReference) -> bool {
        // TODO(wenyuzhao): Object pinning not supported yet.
        false
    }

    /* Line searching */

    pub fn get_next_available_lines(&self, search_start: Line) -> Option<Range<Line>> {
        debug_assert!(!super::BLOCK_ONLY);
        let unavail_state = self.line_unavail_state.load(Ordering::Acquire);
        let current_state = self.line_mark_state.load(Ordering::Acquire);
        let mark_data = search_start.block().line_mark_table();
        let mark_byte_start = mark_data.start + search_start.get_index_within_block();
        let mark_byte_end = mark_data.end;
        let mut mark_byte_cursor = mark_byte_start;
        // Find start
        while mark_byte_cursor < mark_byte_end {
            let mark = unsafe { mark_byte_cursor.load::<u8>() };
            if mark != unavail_state && mark != current_state {
                break;
            }
            mark_byte_cursor = mark_byte_cursor + 1usize;
        }
        if mark_byte_cursor == mark_byte_end { return None }
        let start = Line::forward(search_start, mark_byte_cursor - mark_byte_start);
        // Find limit
        while mark_byte_cursor < mark_byte_end {
            let mark = unsafe { mark_byte_cursor.load::<u8>() };
            if mark == unavail_state || mark == current_state {
                break;
            }
            mark_byte_cursor = mark_byte_cursor + 1usize;
        }
        let end = Line::forward(search_start, mark_byte_cursor - mark_byte_start);
        debug_assert!((start..end).all(|line| !line.is_marked(unavail_state) && !line.is_marked(current_state)));
        return Some(start..end)
    }
}


pub struct PrepareBlockState<VM: VMBinding> {
    pub space: &'static ImmixSpace<VM>,
    pub chunks: Range<Chunk>,
    pub defrag_threshold: Option<usize>,
}

impl<VM: VMBinding> PrepareBlockState<VM> {
    #[inline(always)]
    fn reset_object_mark(chunk: Chunk) {
        if !ImmixSpace::<VM>::HEADER_MARK_BITS {
            side_metadata::bzero_metadata_for_chunk(ImmixSpace::<VM>::OBJECT_MARK_TABLE, chunk.start());
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for PrepareBlockState<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let defrag_threshold = self.defrag_threshold.unwrap_or(0);
        for chunk in self.chunks.clone().filter(|c| self.space.chunk_map.get(*c) == ChunkState::Allocated) {
            Self::reset_object_mark(chunk);
            for block in chunk.blocks() {
                let state = block.get_state();
                if state == BlockState::Unallocated { continue; }
                if super::DEFRAG && defrag_threshold != 0 && !state.is_reusable() && block.get_holes() > defrag_threshold {
                    block.set_as_defrag_source(true);
                } else {
                    block.set_as_defrag_source(false);
                }
                block.set_state(BlockState::Unmarked);
                debug_assert!(!block.get_state().is_reusable());
                debug_assert_ne!(block.get_state(), BlockState::Marked);
            }
        }
    }
}

pub struct ObjectsClosure<'a, E: ProcessEdgesWork>(&'static MMTK<E::VM>, Vec<Address>, &'a mut GCWorker<E::VM>);

impl<'a, E: ProcessEdgesWork> TransitiveClosure for ObjectsClosure<'a, E> {
    #[inline(always)]
    fn process_edge(&mut self, slot: Address) {
        if self.1.len() == 0 {
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
        self.2.add_work(WorkBucketStage::Closure, E::new(new_edges, false, self.0));
    }
}

pub struct ScanObjectsAndMarkLines<Edges: ProcessEdgesWork> {
    buffer: Vec<ObjectReference>,
    #[allow(unused)]
    concurrent: bool,
    immix_space: &'static ImmixSpace<Edges::VM>,
}

impl<Edges: ProcessEdgesWork> ScanObjectsAndMarkLines<Edges> {
    pub fn new(buffer: Vec<ObjectReference>, concurrent: bool, immix_space: &'static ImmixSpace<Edges::VM>) -> Self {
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
            <E::VM as VMBinding>::VMScanning::scan_object(&mut closure, *object, OpaquePointer::UNINITIALIZED);
            if super::MARK_LINE_AT_SCAN_TIME && !super::BLOCK_ONLY && self.immix_space.in_space(*object) {
                self.immix_space.mark_lines(*object);
            }
        }
    }
}