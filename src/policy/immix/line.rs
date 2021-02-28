use std::iter::Step;

use crate::{util::{Address, ObjectReference}, vm::*};
use crate::util::side_metadata::{self, *};

use super::block::Block;



#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Line(Address);

impl Line {
    pub const LOG_BYTES: usize = 8;
    pub const BYTES: usize = 1 << Self::LOG_BYTES;

    pub const RESET_MARK_STATE: u8 = 1;
    pub const MAX_MARK_STATE: u8 = 127;

    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: 0,
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    pub const fn align(address: Address) -> Address {
        address.align_down(Self::BYTES)
    }

    pub const fn is_aligned(address: Address) -> bool {
        Self::align(address).as_usize() == address.as_usize()
    }

    pub const fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    #[inline(always)]
    pub fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self(VM::VMObjectModel::object_start_ref(object).align_down(Self::BYTES))
    }

    pub const fn index(&self, block: Block) -> usize {
        (self.start().as_usize() - block.start().as_usize()) >> Self::LOG_BYTES
    }

    pub const fn block(&self) -> Block {
        Block::from(Block::align(self.0))
    }

    pub const fn start(&self) -> Address {
        self.0
    }

    pub const fn end(&self) -> Address {
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES) }
    }

    #[inline]
    pub fn get_mark(&self) -> u8 {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.start()) as u8 }
    }

    #[inline]
    pub fn mark(&self, state: u8) {
        unsafe { side_metadata::store(Self::MARK_TABLE, self.start(), state as _); }
    }

    #[inline]
    pub fn is_marked(&self, state: u8) -> bool {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.start()) as u8 == state }
    }

    #[inline]
    pub fn mark_lines_for_object<VM: VMBinding>(object: ObjectReference, state: u8) {
        let start = VM::VMObjectModel::object_start_ref(object);
        let end  = start + VM::VMObjectModel::get_current_size(object);
        let start_line = Line::from(Line::align(start));
        let mut end_line = Line::from(Line::align(end));
        if !Line::is_aligned(end) { end_line = Line::forward(end_line, 1) }
        for line in start_line .. end_line {
            line.mark(state)
        }
    }
}

unsafe impl Step for Line {
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        if start < end { return None }
        Some((end.start() - start.start()) >> Line::LOG_BYTES)
    }
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Line::from(start.start() + (count << Line::LOG_BYTES)))
    }
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Line::from(start.start() - (count << Line::LOG_BYTES)))
    }
}