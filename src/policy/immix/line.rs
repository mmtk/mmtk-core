use super::block::Block;
use crate::util::metadata::side_metadata::{self, *};
use crate::{
    util::{Address, ObjectReference},
    vm::*,
};
use std::iter::Step;

/// Data structure to reference a line within an immix block.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Line(Address);

#[allow(clippy::assertions_on_constants)]
impl Line {
    /// Log bytes in block
    pub const LOG_BYTES: usize = 8;
    /// Bytes in block
    pub const BYTES: usize = 1 << Self::LOG_BYTES;

    pub const RESET_MARK_STATE: u8 = 1;
    pub const MAX_MARK_STATE: u8 = 127;

    /// Line mark table (side)
    pub const MARK_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::IX_LINE_MARK;

    /// Align the give address to the line boundary.
    #[inline(always)]
    pub fn align(address: Address) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        address.align_down(Self::BYTES)
    }

    /// Test if the given address is line-aligned
    #[inline(always)]
    pub fn is_aligned(address: Address) -> bool {
        debug_assert!(!super::BLOCK_ONLY);
        Self::align(address).as_usize() == address.as_usize()
    }

    /// Get the line from a given address.
    /// The address must be line-aligned.
    #[inline(always)]
    pub fn from(address: Address) -> Self {
        debug_assert!(!super::BLOCK_ONLY);
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    /// Get the block containing the line.
    #[inline(always)]
    pub fn block(&self) -> Block {
        debug_assert!(!super::BLOCK_ONLY);
        Block::from(Block::align(self.0))
    }

    /// Get line start address
    #[inline(always)]
    pub fn start(&self) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        self.0
    }

    /// Get line index within its containing block.
    #[inline(always)]
    pub fn get_index_within_block(&self) -> usize {
        let addr = self.start();
        addr.get_extent(Block::align(addr)) >> Line::LOG_BYTES
    }

    /// Mark the line. This will update the side line mark table.
    #[inline]
    pub fn mark(&self, state: u8) {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe {
            side_metadata::store(&Self::MARK_TABLE, self.start(), state as _);
        }
    }

    /// Test line mark state.
    #[inline(always)]
    pub fn is_marked(&self, state: u8) -> bool {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe { side_metadata::load(&Self::MARK_TABLE, self.start()) as u8 == state }
    }

    /// Mark all lines the object is spanned to.
    #[inline]
    pub fn mark_lines_for_object<VM: VMBinding>(object: ObjectReference, state: u8) -> usize {
        debug_assert!(!super::BLOCK_ONLY);
        let start = VM::VMObjectModel::object_start_ref(object);
        let end = start + VM::VMObjectModel::get_current_size(object);
        let start_line = Line::from(Line::align(start));
        let mut end_line = Line::from(Line::align(end));
        if !Line::is_aligned(end) {
            end_line = Line::forward(end_line, 1)
        }
        let mut marked_lines = 0;
        for line in start_line..end_line {
            if !line.is_marked(state) {
                marked_lines += 1;
            }
            line.mark(state)
        }
        marked_lines
    }
}

#[allow(clippy::assertions_on_constants)]
impl Step for Line {
    /// Get the number of lines between the given two lines.
    #[inline(always)]
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        debug_assert!(!super::BLOCK_ONLY);
        if start > end {
            return None;
        }
        Some((end.start() - start.start()) >> Line::LOG_BYTES)
    }
    /// result = line_address + count * block_size
    #[inline(always)]
    fn forward(start: Self, count: usize) -> Self {
        debug_assert!(!super::BLOCK_ONLY);
        Self::from(start.start() + (count << Self::LOG_BYTES))
    }
    /// result = line_address + count * block_size
    #[inline(always)]
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        debug_assert!(!super::BLOCK_ONLY);
        if start.start().as_usize() > usize::MAX - (count << Self::LOG_BYTES) {
            return None;
        }
        Some(Self::forward(start, count))
    }
    /// result = line_address + count * block_size
    #[inline(always)]
    fn backward(start: Self, count: usize) -> Self {
        debug_assert!(!super::BLOCK_ONLY);
        Self::from(start.start() - (count << Self::LOG_BYTES))
    }
    /// result = line_address - count * block_size
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        debug_assert!(!super::BLOCK_ONLY);
        if start.start().as_usize() < (count << Self::LOG_BYTES) {
            return None;
        }
        Some(Self::backward(start, count))
    }
}
