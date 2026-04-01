use std::ops::Range;

use super::block::Block;
use crate::util::constants::{LOG_BITS_IN_BYTE, LOG_BYTES_IN_WORD, LOG_MIN_OBJECT_SIZE};
use crate::util::linear_scan::{Region, RegionIterator};
use crate::util::metadata::side_metadata::spec_defs::IX_LINE_REUSE_COUNT;
use crate::util::metadata::side_metadata::*;
use crate::util::rc;
use crate::{
    util::{Address, ObjectReference},
    vm::*,
};
use atomic::Ordering;

/// Data structure to reference a line within an immix block.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Line(Address);

impl Region for Line {
    const LOG_BYTES: usize = 8;

    #[allow(clippy::assertions_on_constants)] // make sure line is not used when BLOCK_ONLY is turned on.
    fn from_aligned_address(address: Address) -> Self {
        debug_assert!(!super::BLOCK_ONLY);
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    fn start(&self) -> Address {
        self.0
    }
}

#[allow(clippy::assertions_on_constants)]
impl Line {
    pub const RESET_MARK_STATE: u8 = 1;
    pub const MAX_MARK_STATE: u8 = 127;

    /// Line mark table (side)
    pub const MARK_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::IX_LINE_MARK;

    /// Align the give address to the line boundary.
    pub fn align(address: Address) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        address.align_down(Self::BYTES)
    }

    /// Test if the given address is line-aligned
    pub fn is_aligned(address: Address) -> bool {
        debug_assert!(!super::BLOCK_ONLY);
        Self::align(address).as_usize() == address.as_usize()
    }

    /// Get the line from a given address.
    /// The address must be line-aligned.
    pub fn from(address: Address) -> Self {
        debug_assert!(!super::BLOCK_ONLY);
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    pub fn of(a: Address) -> Self {
        Self(a.align_down(Self::BYTES))
    }

    pub fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self(VM::VMObjectModel::ref_to_object_start(object).align_down(Self::BYTES))
    }

    /// Get the block containing the line.
    pub fn block(&self) -> Block {
        debug_assert!(!super::BLOCK_ONLY);
        Block::from_unaligned_address(self.0)
    }

    /// Get line start address
    pub const fn start(&self) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        self.0
    }

    pub const fn end(&self) -> Address {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES) }
    }

    /// Get line index within its containing block.
    pub const fn get_index_within_block(&self) -> usize {
        let addr = self.start();
        addr.get_extent(Block::align(addr)) >> Line::LOG_BYTES
    }

    /// Mark the line. This will update the side line mark table.
    pub fn mark(&self, state: u8) {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe {
            Self::MARK_TABLE.store::<u8>(self.start(), state);
        }
    }

    /// Test line mark state.
    pub fn is_marked(&self, state: u8) -> bool {
        debug_assert!(!super::BLOCK_ONLY);
        unsafe { Self::MARK_TABLE.load::<u8>(self.start()) == state }
    }

    pub fn is_marked_by_satb<VM: VMBinding>(&self) -> bool {
        for i in (0..Self::BYTES).step_by(8) {
            let ptr = self.start() + i;
            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .extract_side_spec()
                .load_atomic::<u8>(ptr, Ordering::SeqCst)
                == 1
            {
                return true;
            }
        }
        false
    }

    /// Mark all lines the object is spanned to.
    pub fn mark_lines_for_object<VM: VMBinding>(object: ObjectReference, state: u8) -> usize {
        debug_assert!(!super::BLOCK_ONLY);
        let start = object.to_object_start::<VM>();
        let end = start + VM::VMObjectModel::get_current_size(object);
        let start_line = Line::from_unaligned_address(start);
        let mut end_line = Line::from_unaligned_address(end);
        if !Line::is_aligned(end) {
            end_line = end_line.next();
        }
        let mut marked_lines = 0;
        let iter = RegionIterator::<Line>::new(start_line, end_line);
        for line in iter {
            if !line.is_marked(state) {
                marked_lines += 1;
            }
            line.mark(state)
        }
        marked_lines
    }

    /// Bulk set the local mark bits of a line range.
    ///
    /// This is useful during concurrent marking. By doing this, concurrent marking will
    /// conservatively consider all objects allocated in the line range as live, and the mutator
    /// doesn't need to explicitly mark bump-allocated objects in the fast path.
    pub fn initialize_mark_table_as_marked<VM: VMBinding>(lines: Range<Line>) {
        let meta = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.extract_side_spec();
        let start: *mut u8 = address_to_meta_address(&meta, lines.start.start()).to_mut_ptr();
        let limit: *mut u8 = address_to_meta_address(&meta, lines.end.start()).to_mut_ptr();
        unsafe {
            let bytes = limit.offset_from(start) as usize;
            std::ptr::write_bytes(start, 0xffu8, bytes);
        }
    }

    pub fn inc_reuse_counts<VM: VMBinding>(lines: Range<Line>) {
        let mut l = lines.start;
        while l < lines.end {
            let addr = l.start();
            let count = IX_LINE_REUSE_COUNT.load_atomic::<u8>(addr, Ordering::SeqCst);
            let new_count = if count == u8::MAX { 0 } else { count + 1 };
            IX_LINE_REUSE_COUNT.store_atomic::<u8>(addr, new_count, Ordering::SeqCst);
            l = l.next();
        }
    }

    /// Bulk set line mark states.
    pub fn bulk_set_line_mark_states(line_mark_state: u8, lines: Range<Line>) {
        for line in RegionIterator::<Line>::new(lines.start, lines.end) {
            line.mark(line_mark_state);
        }
    }

    /// Eagerly mark all line mark states and all side mark bits in the gap.
    ///
    /// Useful during concurrent marking.
    pub fn eager_mark_lines<VM: VMBinding>(line_mark_state: u8, lines: Range<Line>) {
        Self::bulk_set_line_mark_states(line_mark_state, lines.clone());
        Self::initialize_mark_table_as_marked::<VM>(lines);
    }

    pub fn clear_field_unlog_table<VM: VMBinding>(lines: Range<Line>) {
        let unlog_bit = *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
            .as_spec()
            .extract_side_spec();
        let log_meta_bits_per_line = Line::LOG_BYTES - LOG_BYTES_IN_WORD as usize
            + if !VM::VMObjectModel::COMPRESSED_PTR_ENABLED {
                0
            } else {
                1
            };
        debug_assert!((1 << log_meta_bits_per_line) >= 8);
        let log_meta_bytes_per_line = log_meta_bits_per_line - LOG_BITS_IN_BYTE as usize;
        // FIXME: Performance
        let start = lines.start.start();
        let meta_start = address_to_meta_address(&unlog_bit, start);
        let meta_bytes =
            Line::steps_between(&lines.start, &lines.end).unwrap() << log_meta_bytes_per_line;
        crate::util::memory::zero(meta_start, meta_bytes)
    }

    pub fn initialize_field_unlog_table_as_unlogged<VM: VMBinding>(lines: Range<Line>) {
        let unlog_bit = *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
            .as_spec()
            .extract_side_spec();
        let log_meta_bits_per_line = Line::LOG_BYTES - LOG_BYTES_IN_WORD as usize
            + if !VM::VMObjectModel::COMPRESSED_PTR_ENABLED {
                0
            } else {
                1
            };
        debug_assert!((1 << log_meta_bits_per_line) >= 8);
        let log_meta_bytes_per_line = log_meta_bits_per_line - LOG_BITS_IN_BYTE as usize;
        // FIXME: Performance
        let start = lines.start.start();
        let meta_start = address_to_meta_address(&unlog_bit, start);
        let meta_bytes =
            Line::steps_between(&lines.start, &lines.end).unwrap() << log_meta_bytes_per_line;
        unsafe {
            std::ptr::write_bytes::<u8>(meta_start.to_mut_ptr(), 0xffu8, meta_bytes);
        }
    }

    pub fn clear_mark_table<VM: VMBinding>(lines: Range<Line>) {
        // FIXME: Performance
        let start = lines.start.start();
        let size = Line::steps_between(&lines.start, &lines.end).unwrap() << Line::LOG_BYTES;
        let mark_bit = VM::VMObjectModel::LOCAL_MARK_BIT_SPEC.extract_side_spec();
        for i in (0..size).step_by(1 << LOG_MIN_OBJECT_SIZE) {
            mark_bit.store_atomic(start + i, 0u8, Ordering::SeqCst);
        }
    }
}

// type UInt<const BITS: usize> =

pub trait UintType: 'static + Sized {
    type Type: 'static + Sized + Copy + Eq + PartialEq;
    fn is_zero(v: Self::Type) -> bool;
}

pub struct Uint<const BITS: usize> {}

impl UintType for Uint<8> {
    type Type = u8;
    fn is_zero(v: Self::Type) -> bool {
        v == 0
    }
}

impl UintType for Uint<16> {
    type Type = u16;
    fn is_zero(v: Self::Type) -> bool {
        v == 0
    }
}

impl UintType for Uint<32> {
    type Type = u32;
    fn is_zero(v: Self::Type) -> bool {
        v == 0
    }
}

impl UintType for Uint<64> {
    type Type = u64;
    fn is_zero(v: Self::Type) -> bool {
        v == 0
    }
}

impl UintType for Uint<128> {
    type Type = u128;
    fn is_zero(v: Self::Type) -> bool {
        v == 0
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct UInt256([u8; 256 / 8]);

impl UintType for Uint<256> {
    type Type = UInt256;
    fn is_zero(v: Self::Type) -> bool {
        v == UInt256([0; 256 / 8])
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct UInt512([u8; 512 / 8]);

impl UintType for Uint<512> {
    type Type = UInt512;
    fn is_zero(v: Self::Type) -> bool {
        v == UInt512([0; 512 / 8])
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct UInt1024([u8; 1024 / 8]);

impl UintType for Uint<1024> {
    type Type = UInt1024;
    fn is_zero(v: Self::Type) -> bool {
        v == UInt1024([0; 1024 / 8])
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct UInt2048([u8; 2048 / 8]);

impl UintType for Uint<2048> {
    type Type = UInt2048;
    fn is_zero(v: Self::Type) -> bool {
        v == UInt2048([0; 2048 / 8])
    }
}

const LOG_BITS_PER_LINE: usize = Line::LOG_BYTES - rc::LOG_MIN_OBJECT_SIZE + rc::LOG_REF_COUNT_BITS;
const BITS_PER_LINE: usize = 1 << LOG_BITS_PER_LINE;
const LOG_BITS_PER_BLOCK: usize =
    Block::LOG_BYTES - rc::LOG_MIN_OBJECT_SIZE + rc::LOG_REF_COUNT_BITS;
const BITS_PER_BLOCK: usize = 1 << LOG_BITS_PER_BLOCK;

pub struct RCArray {
    table: &'static [<Uint<{ BITS_PER_LINE }> as UintType>::Type; BITS_PER_BLOCK / BITS_PER_LINE],
}

impl RCArray {
    pub fn of(block: Block) -> Self {
        Self {
            table: unsafe { &*block.rc_table_start().to_ptr() },
        }
    }

    pub fn is_dead(&self, i: usize) -> bool {
        <Uint<{ BITS_PER_LINE }> as UintType>::is_zero(self.table[i])
    }
}
