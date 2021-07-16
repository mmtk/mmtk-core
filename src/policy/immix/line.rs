use super::block::Block;
use crate::util::metadata::side_metadata::{self, *};
use crate::{
    util::{Address, ObjectReference},
    vm::*,
};
use std::iter::Step;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Line(Address);

impl Line {
    pub const LOG_BYTES: usize = 8;
    pub const BYTES: usize = 1 << Self::LOG_BYTES;

    pub const RESET_MARK_STATE: u8 = 1;
    pub const MAX_MARK_STATE: u8 = 127;

    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize(),
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    #[inline(always)]
    pub fn align(address: Address) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        address.align_down(Self::BYTES)
    }

    #[inline(always)]
    pub fn is_aligned(address: Address) -> bool {
        debug_assert!(!super::BLOCK_ONLY);
        Self::align(address).as_usize() == address.as_usize()
    }

    #[inline(always)]
    pub fn from(address: Address) -> Self {
        debug_assert!(!super::BLOCK_ONLY);
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    #[inline(always)]
    pub fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        debug_assert!(!super::BLOCK_ONLY);
        Self(VM::VMObjectModel::ref_to_address(object).align_down(Self::BYTES))
    }

    #[inline(always)]
    pub fn block(&self) -> Block {
        debug_assert!(!super::BLOCK_ONLY);
        Block::from(Block::align(self.0))
    }

    #[inline(always)]
    pub fn start(&self) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        self.0
    }

    #[inline(always)]
    pub fn end(&self) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES) }
    }

    #[inline(always)]
    pub fn get_index_within_block(&self) -> usize {
        let addr = self.start();
        (addr.as_usize() - Block::align(addr).as_usize()) >> Line::LOG_BYTES
    }

    #[inline]
    pub fn mark(&self, state: u8) {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe {
            side_metadata::store(&Self::MARK_TABLE, self.start(), state as _);
        }
    }

    #[inline(always)]
    pub fn mark_byte_address(&self) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        side_metadata::address_to_meta_address(&Self::MARK_TABLE, self.start())
    }

    #[inline(always)]
    pub fn is_marked(&self, state: u8) -> bool {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe { side_metadata::load(&Self::MARK_TABLE, self.start()) as u8 == state }
    }

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
        return marked_lines;
    }
}

unsafe impl Step for Line {
    #[inline(always)]
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        debug_assert!(!super::BLOCK_ONLY);
        if start > end {
            return None;
        }
        Some((end.start() - start.start()) >> Line::LOG_BYTES)
    }
    #[inline(always)]
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        debug_assert!(!super::BLOCK_ONLY);
        Some(Line::from(start.start() + (count << Line::LOG_BYTES)))
    }
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        debug_assert!(!super::BLOCK_ONLY);
        Some(Line::from(start.start() - (count << Line::LOG_BYTES)))
    }
}
