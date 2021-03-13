use atomic::Ordering;
use crate::{AllocationSemantics, CopyContext, plan::TransitiveClosure, util::{OpaquePointer, constants::{LOG_BYTES_IN_WORD}, heap::FreeListPageResource}};
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
            if !block.is_defrag() {
                block.clear_mark();
            }
            side_metadata::bzero_metadata_for_range(Self::OBJECT_MARK_TABLE, Range { start: block.start(), end: block.end() });
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

        for block in self.block_list.drain_filter(|block| !block.is_marked() || block.is_defrag()) {
            self.pr.release_pages(block.start());
        }

        if !super::BLOCK_ONLY {
            self.reusable_blocks.reset();
            let line_mark_state = self.line_mark_state.load(Ordering::SeqCst);
            for block in &self.block_list {
                debug_assert!(block.is_marked());
                let marked_lines = block.count_marked_lines(line_mark_state);
                debug_assert!(marked_lines > 0);
                if super::DEFRAG && marked_lines <= super::DEFRAG_THRESHOLD {
                    block.mark_as_defrag()
                } else if marked_lines < Block::LINES {
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
        block.clear_mark();
        self.block_list.push(block);
        Some(block)
    }

    pub fn get_reusable_block(&self) -> Option<Block> {
        if !super::BLOCK_ONLY { return None }
        self.reusable_blocks.pop()
    }

    #[inline(always)]
    pub fn trace_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference, semantics: AllocationSemantics, copy_context: &mut impl CopyContext) -> ObjectReference {
        if Block::containing::<VM>(object).is_defrag() {
            self.trace_evacuate_object(trace, object, semantics, copy_context)
        } else {
            self.trace_mark_object(trace, object)
        }
    }

    #[inline(always)]
    pub fn trace_mark_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference) -> ObjectReference {
        if Self::attempt_mark(object) {
            // Mark block and lines
            if !super::BLOCK_ONLY {
                let marked_lines = Line::mark_lines_for_object::<VM>(object, self.line_mark_state.load(Ordering::SeqCst));
                Block::containing::<VM>(object).mark(Some(marked_lines));
            } else {
                Block::containing::<VM>(object).mark(None);
            }
            // Visit node
            trace.process_node(object);
        }
        object
    }

    #[inline]
    pub fn trace_evacuate_object(&self, trace: &mut impl TransitiveClosure, object: ObjectReference, semantics: AllocationSemantics, copy_context: &mut impl CopyContext) -> ObjectReference {
        let forwarding_status = ForwardingWord::attempt_to_forward::<VM>(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_status) {
            ForwardingWord::spin_and_get_forwarded_object::<VM>(object, forwarding_status)
        } else {
            let new_object = ForwardingWord::forward_object::<VM, _>(object, semantics, copy_context);
            trace.process_node(new_object);
            trace!("{:?} => {:?} in block {:?} {}", object, new_object, Block::containing::<VM>(new_object), Block::containing::<VM>(new_object).mark_byte());
            // Mark block and lines
            if !super::BLOCK_ONLY {
                let marked_lines = Line::mark_lines_for_object::<VM>(new_object, self.line_mark_state.load(Ordering::SeqCst));
                Block::containing::<VM>(new_object).mark(Some(marked_lines));
            } else {
                Block::containing::<VM>(new_object).mark(None);
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

    pub fn get_next_available_lines(&self, start: Line) -> Option<Range<Line>> {
        debug_assert!(!super::BLOCK_ONLY);
        let unavail_state = self.line_unavail_state.load(Ordering::SeqCst);
        let current_state = self.line_mark_state.load(Ordering::SeqCst);
        let line_limit = start.block().lines().end;
        let mark_byte_start = start.mark_byte_address();
        let mark_byte_end = Line::backward(line_limit, 1).mark_byte_address() + 1usize;
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
        let start = Line::forward(start, mark_byte_cursor - mark_byte_start);
        // Find limit
        while mark_byte_cursor < mark_byte_end {
            let mark = unsafe { mark_byte_cursor.load::<u8>() };
            if mark == unavail_state || mark == current_state {
                break;
            }
            mark_byte_cursor = mark_byte_cursor + 1usize;
        }
        let end = Line::forward(start, mark_byte_cursor - mark_byte_start);
        debug_assert!((start..end).all(|line| !line.is_marked(unavail_state) && !line.is_marked(current_state)));
        return Some(Range { start, end })
    }
}
