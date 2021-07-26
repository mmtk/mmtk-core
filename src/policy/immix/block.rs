use super::chunk::Chunk;
use super::defrag::MarkHistogram;
use super::line::Line;
use super::ImmixSpace;
use crate::util::constants::*;
use crate::util::metadata::side_metadata::{self, *};
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crossbeam_queue::SegQueue;
use spin::RwLock;
use std::{
    iter::Step,
    ops::Range,
    sync::atomic::{AtomicU8, Ordering},
};

/// The block allocation state.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BlockState {
    /// the block is not allocated.
    Unallocated,
    /// the block is allocated but not marked.
    Unmarked,
    /// the block is allocated and marked.
    Marked,
    /// the block is marked as reusable.
    Reusable { unavailable_lines: u8 },
}

impl BlockState {
    /// Private constant
    const MARK_UNALLOCATED: u8 = 0;
    /// Private constant
    const MARK_UNMARKED: u8 = u8::MAX;
    /// Private constant
    const MARK_MARKED: u8 = u8::MAX - 1;
}

impl From<u8> for BlockState {
    #[inline(always)]
    fn from(state: u8) -> Self {
        match state {
            Self::MARK_UNALLOCATED => BlockState::Unallocated,
            Self::MARK_UNMARKED => BlockState::Unmarked,
            Self::MARK_MARKED => BlockState::Marked,
            unavailable_lines => BlockState::Reusable { unavailable_lines },
        }
    }
}

impl From<BlockState> for u8 {
    #[inline(always)]
    fn from(state: BlockState) -> Self {
        match state {
            BlockState::Unallocated => BlockState::MARK_UNALLOCATED,
            BlockState::Unmarked => BlockState::MARK_UNMARKED,
            BlockState::Marked => BlockState::MARK_MARKED,
            BlockState::Reusable { unavailable_lines } => unavailable_lines,
        }
    }
}

impl BlockState {
    /// Test if the block is reuasable.
    pub const fn is_reusable(&self) -> bool {
        matches!(self, BlockState::Reusable { .. })
    }
}

/// Data structure to reference an immix block.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq)]
pub struct Block(Address);

impl Block {
    /// Log bytes in block
    pub const LOG_BYTES: usize = 15;
    /// Bytes in block
    pub const BYTES: usize = 1 << Self::LOG_BYTES;
    /// Log pages in block
    pub const LOG_PAGES: usize = Self::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;
    /// Pages in block
    pub const PAGES: usize = 1 << Self::LOG_PAGES;
    /// Log lines in block
    pub const LOG_LINES: usize = Self::LOG_BYTES - Line::LOG_BYTES;
    /// Lines in block
    pub const LINES: usize = 1 << Self::LOG_LINES;

    /// Block defrag state table (side)
    pub const DEFRAG_STATE_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: if super::BLOCK_ONLY {
            // If BLOCK_ONLY is set, we do not use any line marktables.
            LOCAL_SIDE_METADATA_BASE_OFFSET
        } else {
            SideMetadataOffset::layout_after(&Line::MARK_TABLE)
        },
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    /// Block mark table (side)
    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Self::DEFRAG_STATE_TABLE),
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    /// Align the address to a block boundary.
    pub const fn align(address: Address) -> Address {
        address.align_down(Self::BYTES)
    }

    /// Get the block from a given address.
    /// The address must be block-aligned.
    #[inline(always)]
    pub fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    /// Get the block containing the given address.
    /// The input address does not need to be aligned.
    #[inline(always)]
    pub fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self(VM::VMObjectModel::ref_to_address(object).align_down(Self::BYTES))
    }

    /// Get block start address
    pub const fn start(&self) -> Address {
        self.0
    }

    /// Get block end address
    pub const fn end(&self) -> Address {
        self.0.add(Self::BYTES)
    }

    /// Get the chunk containing the block.
    #[inline(always)]
    pub fn chunk(&self) -> Chunk {
        Chunk::from(Chunk::align(self.0))
    }

    /// Get the address range of the block's line mark table.
    #[allow(clippy::assertions_on_constants)]
    #[inline(always)]
    pub fn line_mark_table(&self) -> &[AtomicU8; Block::LINES] {
        debug_assert!(!super::BLOCK_ONLY);
        let start = side_metadata::address_to_meta_address(&Line::MARK_TABLE, self.start());
        // # Safety
        // The metadata memory is assumed to be mapped when accessing.
        unsafe { &*start.to_ptr() }
    }

    #[inline(always)]
    fn mark_byte(&self) -> &AtomicU8 {
        // # Safety
        // The metadata memory is assumed to be mapped when accessing.
        unsafe {
            &*side_metadata::address_to_meta_address(&Self::MARK_TABLE, self.start())
                .to_mut_ptr::<AtomicU8>()
        }
    }

    /// Get block mark state.
    #[inline(always)]
    pub fn get_state(&self) -> BlockState {
        self.mark_byte().load(Ordering::Acquire).into()
    }

    /// Set block mark state.
    #[inline(always)]
    pub fn set_state(&self, state: BlockState) {
        self.mark_byte().store(state.into(), Ordering::Release)
    }

    // Defrag byte

    const DEFRAG_SOURCE_STATE: u8 = u8::MAX;

    #[inline(always)]
    fn defrag_byte(&self) -> &AtomicU8 {
        // # Safety
        // The metadata memory is assumed to be mapped when accessing.
        unsafe {
            &*side_metadata::address_to_meta_address(&Self::DEFRAG_STATE_TABLE, self.start())
                .to_mut_ptr::<AtomicU8>()
        }
    }

    /// Test if the block is marked for defragmentation.
    #[inline(always)]
    pub fn is_defrag_source(&self) -> bool {
        let byte = self.defrag_byte().load(Ordering::Acquire);
        debug_assert!(byte == 0 || byte == Self::DEFRAG_SOURCE_STATE);
        byte == Self::DEFRAG_SOURCE_STATE
    }

    /// Mark the block for defragmentation.
    #[inline(always)]
    pub fn set_as_defrag_source(&self, defrag: bool) {
        if cfg!(debug_assertions) && defrag {
            debug_assert!(!self.get_state().is_reusable());
        }
        self.defrag_byte().store(
            if defrag { Self::DEFRAG_SOURCE_STATE } else { 0 },
            Ordering::Release,
        );
    }

    /// Record the number of holes in the block.
    #[inline(always)]
    pub fn set_holes(&self, holes: usize) {
        self.defrag_byte().store(holes as _, Ordering::Release);
    }

    /// Get the number of holes.
    #[inline(always)]
    pub fn get_holes(&self) -> usize {
        let byte = self.defrag_byte().load(Ordering::Acquire);
        debug_assert_ne!(byte, Self::DEFRAG_SOURCE_STATE);
        byte as usize
    }

    /// Initialize a clean block after acquired from page-resource.
    #[inline]
    pub fn init(&self, copy: bool) {
        self.set_state(if copy {
            BlockState::Marked
        } else {
            BlockState::Unmarked
        });
        self.defrag_byte().store(0, Ordering::Release);
    }

    /// Deinitalize a block before releasing.
    #[inline]
    pub fn deinit(&self) {
        self.set_state(BlockState::Unallocated);
    }

    /// Get the range of lines within the block.
    #[allow(clippy::assertions_on_constants)]
    #[inline(always)]
    pub fn lines(&self) -> Range<Line> {
        debug_assert!(!super::BLOCK_ONLY);
        Line::from(self.start())..Line::from(self.end())
    }

    /// Sweep this block.
    /// Return true if the block is swept.
    #[inline(always)]
    pub fn sweep<VM: VMBinding>(
        &self,
        space: &ImmixSpace<VM>,
        mark_histogram: &mut MarkHistogram,
        line_mark_state: Option<u8>,
    ) -> bool {
        if super::BLOCK_ONLY {
            match self.get_state() {
                BlockState::Unallocated => false,
                BlockState::Unmarked => {
                    // Release the block if it is allocated but not marked by the current GC.
                    space.release_block(*self);
                    true
                }
                BlockState::Marked => {
                    // The block is live.
                    false
                }
                _ => unreachable!(),
            }
        } else {
            // Calculate number of marked lines and holes.
            let mut marked_lines = 0;
            let mut holes = 0;
            let mut prev_line_is_marked = true;
            let line_mark_state = line_mark_state.unwrap();

            for line in self.lines() {
                if line.is_marked(line_mark_state) {
                    marked_lines += 1;
                    prev_line_is_marked = true;
                } else {
                    if prev_line_is_marked {
                        holes += 1;
                    }
                    prev_line_is_marked = false;
                }
            }

            if marked_lines == 0 {
                // Release the block if non of its lines are marked.
                space.release_block(*self);
                true
            } else {
                // There are some marked lines. Keep the block live.
                if marked_lines != Block::LINES {
                    // There are holes. Mark the block as reusable.
                    self.set_state(BlockState::Reusable {
                        unavailable_lines: marked_lines as _,
                    });
                    space.reusable_blocks.push(*self)
                } else {
                    // Clear mark state.
                    self.set_state(BlockState::Unmarked);
                }
                // Update mark_histogram
                mark_histogram[holes] += marked_lines;
                // Record number of holes in block side metadata.
                self.set_holes(holes);
                false
            }
        }
    }
}

unsafe impl Step for Block {
    /// Get the number of blocks between the given two blocks.
    #[inline(always)]
    #[allow(clippy::assertions_on_constants)]
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        debug_assert!(!super::BLOCK_ONLY);
        if start > end {
            return None;
        }
        Some((end.start() - start.start()) >> Self::LOG_BYTES)
    }
    /// result = block_address + count * block_size
    #[inline(always)]
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        if start.start().as_usize() > usize::MAX - (count << Self::LOG_BYTES) {
            return None;
        }
        Some(Self::from(start.start() + (count << Self::LOG_BYTES)))
    }
    /// result = block_address - count * block_size
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        if start.start().as_usize() < (count << Self::LOG_BYTES) {
            return None;
        }
        Some(Self::from(start.start() - (count << Self::LOG_BYTES)))
    }
}

/// A non-block single-linked list to store blocks.
#[derive(Default)]
pub struct BlockList {
    queue: RwLock<SegQueue<Block>>,
}

impl BlockList {
    /// Get number of blocks in this list.
    #[inline]
    pub fn len(&self) -> usize {
        self.queue.read().len()
    }

    /// Add a block to the list.
    #[inline]
    pub fn push(&self, block: Block) {
        self.queue.read().push(block)
    }

    /// Pop a block out of the list.
    #[inline]
    pub fn pop(&self) -> Option<Block> {
        self.queue.read().pop()
    }

    /// Clear the list.
    #[inline]
    pub fn reset(&self) {
        *self.queue.write() = SegQueue::new()
    }

    /// Get an array of all reusable blocks stored in this BlockList.
    #[inline]
    pub fn get_blocks(&self) -> Vec<Block> {
        let mut queue = self.queue.write();
        let mut blocks = Vec::with_capacity(queue.len());
        let new_queue = SegQueue::new();
        while let Some(block) = queue.pop() {
            new_queue.push(block);
            blocks.push(block);
        }
        *queue = new_queue;
        blocks
    }
}
