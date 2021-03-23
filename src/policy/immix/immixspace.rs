use atomic::Ordering;
use crate::{AllocationSemantics, CopyContext, MMTK, plan::TransitiveClosure, scheduler::{WorkBucketStage, GCWorkBucket}, util::{OpaquePointer, constants::{LOG_BYTES_IN_WORD}, heap::FreeListPageResource}};
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
use std::{cell::UnsafeCell, iter::Step, ops::Range, sync::atomic::{AtomicBool, AtomicU8}};
use super::{block::*, chunk::{ChunkMap, ChunkState}, defrag::Defrag};
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
        !self.from_space()
    }
    fn initialize_header(&self, _object: ObjectReference, _alloc: bool) {}
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
        Self::OBJECT_MARK_TABLE.accumulated_size()
    }
}

impl<VM: VMBinding> ImmixSpace<VM> {
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
        let start = common.start;
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

    pub fn prepare(&self) {
        if !super::BLOCK_ONLY {
            self.defrag.prepare(self);
        }
        for chunk in self.chunk_map.allocated_chunks() {
            // Clear object marking data
            side_metadata::bzero_metadata_for_chunk(Self::OBJECT_MARK_TABLE, chunk.start());
            // Clear block marking data
            for block in chunk.blocks() {
                if block.get_state() != BlockState::Unallocated {
                    block.set_state(BlockState::Unmarked);
                }
            }
        }
        if !super::BLOCK_ONLY {
            self.line_mark_state.fetch_add(1, Ordering::SeqCst);
            if self.line_mark_state.load(Ordering::SeqCst) > Line::MAX_MARK_STATE {
                self.line_mark_state.store(Line::RESET_MARK_STATE, Ordering::SeqCst);
            }
        }
        self.in_collection.store(true, Ordering::SeqCst);
    }

    pub fn release(&'static self, mmtk: &'static MMTK<VM>) {
        if !super::BLOCK_ONLY {
            self.line_unavail_state.store(self.line_mark_state.load(Ordering::SeqCst), Ordering::SeqCst);
        }

        if !super::BLOCK_ONLY {
            self.reusable_blocks.reset();
        }

        let work_packets = self.chunk_map.generate_sweep_tasks(self, mmtk);
        mmtk.scheduler.work_buckets[WorkBucketStage::Release].bulk_add(GCWorkBucket::<VM>::DEFAULT_PRIORITY, work_packets);

        self.in_collection.store(false, Ordering::SeqCst);
        if !super::BLOCK_ONLY {
            self.defrag.release(self)
        }
    }

    pub fn release_block(&self, block: Block) {
        block.deinit();
        self.pr.release_pages(block.start());
    }

    pub fn get_clean_block(&self, tls: OpaquePointer) -> Option<Block> {
        let block_address = self.acquire(tls, Block::PAGES);
        if block_address.is_zero() { return None }
        let block = Block::from(block_address);
        block.init();
        self.chunk_map.set(block.chunk(), ChunkState::Allocated);
        Some(block)
    }

    pub fn get_reusable_block(&self) -> Option<Block> {
        if super::BLOCK_ONLY { return None }
        let result = self.reusable_blocks.pop();
        if let Some(block) = result {
            block.init();
        }
        result
    }

    #[inline(always)]
    pub fn fast_trace_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference) -> ObjectReference {
        self.trace_mark_object(trace, object)
    }

    #[inline(always)]
    pub fn trace_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference, semantics: AllocationSemantics, copy_context: &mut impl CopyContext) -> ObjectReference {
        if Block::containing::<VM>(object).is_defrag_source() {
            self.trace_evacuate_object(trace, object, semantics, copy_context)
        } else {
            self.trace_mark_object(trace, object)
        }
    }

    #[inline(always)]
    pub fn trace_mark_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference) -> ObjectReference {
        // println!("trace_mark_object {:?}", object);
        if Self::attempt_mark(object) {
            // Mark block and lines
            if !super::BLOCK_ONLY {
                let _marked_lines = Line::mark_lines_for_object::<VM>(object, self.line_mark_state.load(Ordering::SeqCst));
                Block::containing::<VM>(object).set_state(BlockState::Marked);
            } else {
                Block::containing::<VM>(object).set_state(BlockState::Marked);
            }
            // Visit node
            trace.process_node(object);
        }
        object
    }

    #[inline]
    pub fn trace_evacuate_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference, semantics: AllocationSemantics, copy_context: &mut impl CopyContext) -> ObjectReference {
        debug_assert_eq!(Block::containing::<VM>(object).get_state(), BlockState::Unmarked);
        let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
            ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status)
        } else {
            let new_object = ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context);
            trace.process_node(new_object);
            // trace!("{:?} => {:?} in block {:?}", object, new_object, Block::containing::<VM>(new_object));
            // Mark block and lines
            if !super::BLOCK_ONLY {
                let _marked_lines = Line::mark_lines_for_object::<VM>(new_object, self.line_mark_state.load(Ordering::SeqCst));
                Block::containing::<VM>(new_object).set_state(BlockState::Marked);
            } else {
                Block::containing::<VM>(new_object).set_state(BlockState::Marked);
            }
            new_object
        }
    }

    /* Object Marking */

    const OBJECT_MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: Block::MARK_TABLE.accumulated_size(),
        log_num_of_bits: 0,
        log_min_obj_size: LOG_BYTES_IN_WORD as usize,
    };

    #[inline(always)]
    fn attempt_mark(object: ObjectReference) -> bool {
        side_metadata::compare_exchange_atomic(Self::OBJECT_MARK_TABLE, VM::VMObjectModel::ref_to_address(object), 0, 1)
    }

    /* Line searching */

    pub fn get_next_available_lines(&self, search_start: Line) -> Option<Range<Line>> {
        debug_assert!(!super::BLOCK_ONLY);
        let unavail_state = self.line_unavail_state.load(Ordering::SeqCst);
        let current_state = self.line_mark_state.load(Ordering::SeqCst);
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
