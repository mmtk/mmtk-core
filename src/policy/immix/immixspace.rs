use atomic::Ordering;
use crate::{plan::TransitiveClosure, util::{OpaquePointer, constants::{LOG_BYTES_IN_WORD}, heap::FreeListPageResource}};
use crate::policy::space::SpaceOptions;
use crate::policy::space::{CommonSpace, Space, SFT};
use crate::util::forwarding_word as ForwardingWord;
use crate::util::heap::layout::heap_layout::{Mmapper, VMMap};
use crate::util::conversions;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::heap::PageResource;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::util::side_metadata::{self, *};
use std::{cell::UnsafeCell, iter::Step, ops::Range, sync::atomic::{AtomicBool, AtomicU8}};
use super::block::*;
use super::line::*;



pub struct ImmixSpace<VM: VMBinding> {
    common: UnsafeCell<CommonSpace<VM>>,
    pr: FreeListPageResource<VM>,
    block_list: BlockList,
    line_mark_state: AtomicU8,
    line_unavail_state: AtomicU8,
    in_collection: AtomicBool,
    reusable_blocks: BlockList,
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
        println!("Init Space {:?}", self as *const _);
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
        ImmixSpace {
            pr: if common.vmrequest.is_discontiguous() {
                FreeListPageResource::new_discontiguous(0, vm_map)
            } else {
                FreeListPageResource::new_contiguous(common.start, common.extent, 0, vm_map)
            },
            common: UnsafeCell::new(common),
            block_list: BlockList::new(),
            line_mark_state: AtomicU8::new(Line::RESET_MARK_STATE),
            line_unavail_state: AtomicU8::new(Line::RESET_MARK_STATE),
            in_collection: AtomicBool::new(false),
            reusable_blocks: BlockList::new(),
        }
    }

    pub fn defrag_headroom_pages(&self) -> usize {
        self.pr.reserved_pages() * 2 / 100
    }

    pub fn prepare(&self) {
        for block in &self.block_list {
            block.clear_mark();
            // TODO: clear metadata for a block only
            side_metadata::bzero_metadata_for_chunk(Self::OBJECT_MARK_TABLE, conversions::chunk_align_down(block.start()))
        }
        if !super::BLOCK_ONLY {
            self.line_mark_state.fetch_add(1, Ordering::SeqCst);
            if self.line_mark_state.load(Ordering::SeqCst) > Line::MAX_MARK_STATE {
                self.line_mark_state.store(Line::RESET_MARK_STATE, Ordering::SeqCst);
            }
        }
        self.in_collection.store(true, Ordering::SeqCst);
    }

    pub fn release(&self) {
        if !super::BLOCK_ONLY {
            self.line_unavail_state.store(self.line_mark_state.load(Ordering::SeqCst), Ordering::SeqCst);
        }

        for block in self.block_list.drain_filter(|block| !block.is_marked()) {
            self.pr.release_pages(block.start());
        }

        if !super::BLOCK_ONLY {
            self.reusable_blocks.reset();
            for block in &self.block_list {
                let mut marked_lines = 0;
                for line in block.lines() {
                    if line.is_marked(self.line_mark_state.load(Ordering::SeqCst)) {
                        marked_lines += 1;
                    }
                }
                debug_assert!(block.is_marked());
                debug_assert!(marked_lines > 0);
                if marked_lines < Block::LINES {
                    self.reusable_blocks.push(block);
                }
            }
        }

        self.in_collection.store(false, Ordering::SeqCst);
    }

    pub fn get_clean_block(&self, tls: OpaquePointer) -> Option<Block> {
        let block_address = self.acquire(tls, Block::PAGES);
        if block_address.is_zero() { return None }
        let block = Block::from(block_address);
        self.block_list.push(block);
        Some(block)
    }

    pub fn get_reusable_block(&self) -> Option<Block> {
        if !super::BLOCK_ONLY { return None }
        self.reusable_blocks.pop()
    }

    #[inline]
    pub fn trace_mark_object<T: TransitiveClosure>(&self, trace: &mut T, object: ObjectReference) -> ObjectReference {
        if Self::attempt_mark(object) {
            // Mark block
            Block::containing::<VM>(object).mark();
            if !super::BLOCK_ONLY {
                Line::mark_lines_for_object::<VM>(object, self.line_mark_state.load(Ordering::SeqCst));
            }
            // Visit node
            trace.process_node(object);
        }
        object
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
        side_metadata::compare_exchange_atomic(Self::OBJECT_MARK_TABLE, object.to_address(), 0, 1)
    }

    /* Line searching */

    pub fn get_next_available_lines(&self, start: Line) -> Option<Range<Line>> {
        debug_assert!(!super::BLOCK_ONLY);
        let block = start.block();
        let line_limit = block.lines().end;
        let mut line_cursor = start;
        let unavail_state = self.line_unavail_state.load(Ordering::SeqCst);
        // Find start
        while line_cursor < line_limit {
            if !line_cursor.is_marked(unavail_state) {
                break;
            }
            line_cursor = Line::forward(line_cursor, 1);
        }
        if line_cursor == line_limit { return None }
        let start = line_cursor;
        // Find limit
        while line_cursor < line_limit {
            if line_cursor.is_marked(unavail_state) {
                break;
            }
            line_cursor = Line::forward(line_cursor, 1);
        }
        let end = line_cursor;
        debug_assert!((start..end).all(|line| !line.is_marked(unavail_state)));
        return Some(Range { start, end })
    }
}
