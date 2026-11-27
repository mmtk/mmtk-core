// adapted from Immix

use atomic::Ordering;

use super::BlockList;
use super::MarkSweepSpace;
use crate::util::constants::{BYTES_IN_ADDRESS, LOG_BYTES_IN_PAGE};
use crate::util::heap::chunk_map::*;
use crate::util::linear_scan::Region;
use crate::util::linear_scan::UnstraddlableRegion;
use crate::util::metadata::MetadataSpec;
use crate::util::object_enum::BlockMayHaveObjects;
use crate::vm::ObjectModel;
use crate::{
    util::{
        metadata::side_metadata::SideMetadataSpec, Address, ObjectReference, OpaquePointer,
        VMThread,
    },
    vm::VMBinding,
};

use std::num::NonZeroUsize;

/// A 64KB region for MiMalloc.
/// This is also known as MiMalloc page. We try to avoid getting confused with the OS 4K page. So we call it block.
/// This type always holds a non-zero address to refer to a block. The underlying `NonZeroUsize` type ensures the
/// size of `Option<Block>` is the same as `Block` itself.
// TODO: If we actually use the first block, we would need to turn the type into `Block(Address)`, and use `None` and
// `Block(Address::ZERO)` to differentiate those.
#[derive(Clone, Copy, PartialOrd, PartialEq)]
#[repr(transparent)]
pub struct Block(NonZeroUsize);

impl std::fmt::Debug for Block {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Block(0x{:x})", self.0)
    }
}

impl Region for Block {
    const LOG_BYTES: usize = 16;

    fn from_aligned_address(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        debug_assert!(!address.is_zero());
        Self(unsafe { NonZeroUsize::new_unchecked(address.as_usize()) })
    }

    fn start(&self) -> Address {
        unsafe { Address::from_usize(self.0.get()) }
    }
}

/// An objects cannot straddle multiple native  blocks.
impl UnstraddlableRegion for Block {}

impl BlockMayHaveObjects for Block {
    fn may_have_objects(&self) -> bool {
        self.get_state() != BlockState::Unallocated
    }
}

impl Block {
    /// Log pages in block
    pub const LOG_PAGES: usize = Self::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;

    pub const METADATA_SPECS: [SideMetadataSpec; 7] = [
        Self::MARK_TABLE,
        Self::NEXT_BLOCK_TABLE,
        Self::PREV_BLOCK_TABLE,
        Self::FREE_LIST_TABLE,
        Self::SIZE_TABLE,
        Self::BLOCK_LIST_TABLE,
        Self::TLS_TABLE,
    ];

    /// Block mark table (side)
    pub const MARK_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_MARK;

    pub const NEXT_BLOCK_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_NEXT;

    pub const PREV_BLOCK_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_PREV;

    pub const FREE_LIST_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_FREE;

    // needed for non GC context
    #[cfg(feature = "malloc_native_mimalloc")]
    pub const LOCAL_FREE_LIST_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_LOCAL_FREE;

    #[cfg(feature = "malloc_native_mimalloc")]
    pub const THREAD_FREE_LIST_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_THREAD_FREE;

    pub const SIZE_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_SIZE;

    pub const BLOCK_LIST_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_LIST;

    pub const TLS_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_TLS;

    pub fn load_free_list(&self) -> Address {
        unsafe { Address::from_usize(Block::FREE_LIST_TABLE.load::<usize>(self.start())) }
    }

    pub fn store_free_list(&self, free_list: Address) {
        unsafe { Block::FREE_LIST_TABLE.store::<usize>(self.start(), free_list.as_usize()) }
    }

    #[cfg(feature = "malloc_native_mimalloc")]
    pub fn load_local_free_list(&self) -> Address {
        unsafe { Address::from_usize(Block::LOCAL_FREE_LIST_TABLE.load::<usize>(self.start())) }
    }

    #[cfg(feature = "malloc_native_mimalloc")]
    pub fn store_local_free_list(&self, local_free: Address) {
        unsafe { Block::LOCAL_FREE_LIST_TABLE.store::<usize>(self.start(), local_free.as_usize()) }
    }

    #[cfg(feature = "malloc_native_mimalloc")]
    pub fn load_thread_free_list(&self) -> Address {
        unsafe {
            Address::from_usize(
                Block::THREAD_FREE_LIST_TABLE.load_atomic::<usize>(self.start(), Ordering::SeqCst),
            )
        }
    }

    #[cfg(feature = "malloc_native_mimalloc")]
    pub fn store_thread_free_list(&self, thread_free: Address) {
        unsafe {
            Block::THREAD_FREE_LIST_TABLE.store::<usize>(self.start(), thread_free.as_usize())
        }
    }

    #[cfg(feature = "malloc_native_mimalloc")]
    pub fn cas_thread_free_list(&self, old_thread_free: Address, new_thread_free: Address) -> bool {
        Block::THREAD_FREE_LIST_TABLE
            .compare_exchange_atomic::<usize>(
                self.start(),
                old_thread_free.as_usize(),
                new_thread_free.as_usize(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    pub fn load_prev_block(&self) -> Option<Block> {
        let prev = unsafe { Block::PREV_BLOCK_TABLE.load::<usize>(self.start()) };
        NonZeroUsize::new(prev).map(Block)
    }

    pub fn load_next_block(&self) -> Option<Block> {
        let next = unsafe { Block::NEXT_BLOCK_TABLE.load::<usize>(self.start()) };
        NonZeroUsize::new(next).map(Block)
    }

    pub fn store_next_block(&self, next: Block) {
        unsafe {
            Block::NEXT_BLOCK_TABLE.store::<usize>(self.start(), next.start().as_usize());
        }
    }

    pub fn clear_next_block(&self) {
        unsafe {
            Block::NEXT_BLOCK_TABLE.store::<usize>(self.start(), 0);
        }
    }

    pub fn store_prev_block(&self, prev: Block) {
        unsafe {
            Block::PREV_BLOCK_TABLE.store::<usize>(self.start(), prev.start().as_usize());
        }
    }

    pub fn clear_prev_block(&self) {
        unsafe {
            Block::PREV_BLOCK_TABLE.store::<usize>(self.start(), 0);
        }
    }

    pub fn store_block_list(&self, block_list: &BlockList) {
        let block_list_usize: usize = block_list as *const BlockList as usize;
        unsafe {
            Block::BLOCK_LIST_TABLE.store::<usize>(self.start(), block_list_usize);
        }
    }

    pub fn load_block_list(&self) -> *mut BlockList {
        let block_list =
            Block::BLOCK_LIST_TABLE.load_atomic::<usize>(self.start(), Ordering::SeqCst);
        block_list as *mut BlockList
    }

    pub fn load_block_cell_size(&self) -> usize {
        Block::SIZE_TABLE.load_atomic::<usize>(self.start(), Ordering::SeqCst)
    }

    pub fn store_block_cell_size(&self, size: usize) {
        debug_assert_ne!(size, 0);
        unsafe { Block::SIZE_TABLE.store::<usize>(self.start(), size) }
    }

    pub fn store_tls(&self, tls: VMThread) {
        let tls_usize: usize = tls.0.to_address().as_usize();
        unsafe { Block::TLS_TABLE.store(self.start(), tls_usize) }
    }

    pub fn load_tls(&self) -> VMThread {
        let tls = Block::TLS_TABLE.load_atomic::<usize>(self.start(), Ordering::SeqCst);
        VMThread(OpaquePointer::from_address(unsafe {
            Address::from_usize(tls)
        }))
    }

    /// Take the first interval of free cells, `[start, end)`, off this block's free list.
    ///
    /// The free list is a list of intervals of adjacent free cells:
    /// an allocator bumps through an interval instead of popping one cell at a time. See
    /// [`IntervalListBuilder`] for the encoding.
    pub fn pop_interval(&self) -> Option<(Address, Address)> {
        let head = self.load_free_list();
        if head.is_zero() {
            return None;
        }
        let (end, next) = read_interval(head, self.load_block_cell_size());
        debug_assert!(
            head < end && end <= self.end(),
            "bad interval {head}..{end} in {self:?}"
        );
        debug_assert!(
            next.is_zero() || (self.includes_address(next) && (next >= end || next < head)),
            "bad next interval {next} after {head}..{end} in {self:?}"
        );
        self.store_free_list(next);
        Some((head, end))
    }

    /// Put the interval `[start, end)` back at the front of this block's free list.
    pub fn push_interval(&self, start: Address, end: Address) {
        debug_assert!(start < end && self.includes_address(start) && end <= self.end());
        write_interval(
            start,
            end,
            self.load_block_cell_size(),
            self.load_free_list(),
        );
        self.store_free_list(start);
    }

    pub fn has_free_cells(&self) -> bool {
        !self.load_free_list().is_zero()
    }

    /// Get block mark state.
    pub fn get_state(&self) -> BlockState {
        let byte = Self::MARK_TABLE.load_atomic::<u8>(self.start(), Ordering::SeqCst);
        byte.into()
    }

    /// Set block mark state.
    pub fn set_state(&self, state: BlockState) {
        let state = u8::from(state);
        Self::MARK_TABLE.store_atomic::<u8>(self.start(), state, Ordering::SeqCst);
    }

    /// Release this block if it is unmarked. Return true if the block is released.
    pub fn attempt_release<VM: VMBinding>(self, space: &MarkSweepSpace<VM>) -> bool {
        match self.get_state() {
            // We should not have unallocated blocks in a block list
            BlockState::Unallocated => unreachable!(),
            BlockState::Unmarked => {
                let block_list = self.load_block_list();
                unsafe { &mut *block_list }.remove(self);
                space.release_block(self);
                true
            }
            BlockState::Marked => {
                // The block is live.
                false
            }
        }
    }

    /// Sweep the block. This is done either lazily in the allocation phase, or eagerly at the end of a GC.
    pub fn sweep<VM: VMBinding>(&self, space: &MarkSweepSpace<VM>) {
        let coalesce = space.coalesce_free_cells();
        // The important point here is that we need to distinguish cell address, allocation address, and object reference.
        // We only know cell addresses here. We do not know the allocation address, and we also do not know the object reference.
        // The mark bit is set for object references, and we need to use the mark bit to decide whether a cell is live or not.

        // We haven't implemented for malloc/free cases, for which we do not have mark bit. We could use valid object bit instead.
        if cfg!(feature = "malloc_native_mimalloc") {
            unimplemented!()
        }

        // With contiguous side mark bits, we can skip over dead cells a metadata word at a time.
        if let MetadataSpec::OnSide(mark_bits) = *VM::VMObjectModel::LOCAL_MARK_BIT_SPEC {
            if mark_bits.uses_contiguous_side_metadata() && mark_bits.log_num_of_bits == 0 {
                return self.side_mark_sweep::<VM>(&mark_bits, coalesce);
            }
        }

        // Check if we can treat it as the simple case: cell address === object reference.
        // If the binding does not use allocation offset, and they use the same allocation alignment which the cell size is aligned to,
        // then we have cell address === allocation address.
        // Furthermore, if the binding does not have an offset between allocation and object reference, then allocation address === cell address.
        if !VM::USE_ALLOCATION_OFFSET
            && VM::MAX_ALIGNMENT == VM::MIN_ALIGNMENT
            && crate::util::conversions::raw_is_aligned(
                self.load_block_cell_size(),
                VM::MAX_ALIGNMENT,
            )
            && VM::VMObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS
        {
            // In this case, we can use the simplest and the most efficicent sweep.
            self.simple_sweep::<VM>(coalesce)
        } else {
            // Otherwise we fallback to a generic but slow sweep. This roughly has ~10% mutator overhead for lazy sweeping.
            self.naive_brute_force_sweep::<VM>(coalesce)
        }
    }

    /// Sweep by visiting only the set mark bits: the cells between two live cells are free. The
    /// cost depends on the number of live objects, not on the number of cells.
    fn side_mark_sweep<VM: VMBinding>(&self, mark_bits: &SideMetadataSpec, coalesce: bool) {
        let cell_size = self.load_block_cell_size();
        debug_assert_ne!(cell_size, 0);
        let offset = VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND;
        let cells_end = self.start() + (Block::BYTES / cell_size) * cell_size;
        let mut intervals = IntervalListBuilder::new(cell_size, coalesce);
        // The start of the first cell that is not yet known to be live or free.
        let mut next = self.start();
        let mut free = |start: Address, end: Address| {
            // We don't know where the dead object references are, so we bulk zero the cells.
            #[cfg(feature = "vo_bit")]
            crate::util::metadata::vo_bit::bzero_vo_bit(start, end - start);
            intervals.add_interval(start, end);
        };

        // An object reference is inside its cell, at least `offset` bytes from the cell start.
        mark_bits.scan_non_zero_values::<u8>(
            self.start().offset(offset),
            self.end(),
            &mut |object| {
                let cell_index = (object.offset(-offset) - self.start()) / cell_size;
                let cell = self.start() + cell_index * cell_size;
                if cell >= next {
                    if cell > next {
                        free(next, cell);
                    }
                    next = cell + cell_size;
                }
            },
        );
        if next < cells_end {
            free(next, cells_end);
        }

        self.store_free_list(intervals.finish());
    }

    /// This implementation uses object reference and cell address interchangably. This is not correct for most cases.
    /// However, in certain cases, such as OpenJDK, this is correct, and efficient. See the sweep method for the invariants
    /// that we need to use this method correctly.
    fn simple_sweep<VM: VMBinding>(&self, coalesce: bool) {
        let cell_size = self.load_block_cell_size();
        debug_assert_ne!(cell_size, 0);
        let mut cell = self.start();
        let mut intervals = IntervalListBuilder::new(cell_size, coalesce);
        while cell + cell_size <= self.start() + Block::BYTES {
            // The invariants we checked earlier ensures that we can use cell and object reference interchangably
            // We may not really have an object in this cell, but if we do, this object reference is correct.
            // About unsafe: We know `cell` is non-zero here.
            let potential_object = unsafe { ObjectReference::from_raw_address_unchecked(cell) };

            if !VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .is_marked::<VM>(potential_object, Ordering::SeqCst)
            {
                // clear VO bit if it is ever set. It is possible that the VO bit is never set for this cell (i.e. there was no object in this cell before this GC),
                // we unset the bit anyway.
                #[cfg(feature = "vo_bit")]
                crate::util::metadata::vo_bit::unset_vo_bit_nocheck(potential_object);
                intervals.add(cell);
            }
            cell += cell_size;
        }

        self.store_free_list(intervals.finish());
    }

    /// This is a naive implementation that is inefficient but should be correct.
    /// In this implementation, we simply go through each possible object
    /// reference and see if it has the mark bit set. If we find mark bit, that means the cell is alive. If we didn't find
    /// the mark bit in the entire cell, it means the cell is dead.
    fn naive_brute_force_sweep<VM: VMBinding>(&self, coalesce: bool) {
        use crate::util::constants::MIN_OBJECT_SIZE;

        // Cell size for this block.
        let cell_size = self.load_block_cell_size();
        // Current cell
        let mut cell = self.start();
        // The free list being built
        let mut intervals = IntervalListBuilder::new(cell_size, coalesce);
        // Current cursor
        let mut cursor = cell;

        debug!("Sweep block {:?}, cell size {}", self, cell_size);

        while cell + cell_size <= self.end() {
            // possible object ref
            let potential_object_ref = unsafe {
                // We know cursor plus an offset cannot be 0.
                ObjectReference::from_raw_address_unchecked(
                    cursor + VM::VMObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND,
                )
            };
            trace!(
                "{:?}: cell = {}, cursor = {}, potential object = {}",
                self,
                cell,
                cursor,
                potential_object_ref
            );

            if VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
                .is_marked::<VM>(potential_object_ref, Ordering::SeqCst)
            {
                debug!("{:?} Live cell: {}", self, cell);
                // If the mark bit is set, the cell is alive.
                // We directly jump to the end of the cell.
                cell += cell_size;
                cursor = cell;
            } else {
                // If the mark bit is not set, we don't know if the cell is alive or not. We keep search for the mark bit.
                cursor += MIN_OBJECT_SIZE;

                if cursor >= cell + cell_size {
                    // We now stepped to the next cell. This means we did not find mark bit in the current cell, and we can add this cell to free list.
                    debug!("{:?} Free cell: {}", self, cell);

                    // Clear VO bit: we don't know where the object reference actually is, so we bulk zero the cell.
                    #[cfg(feature = "vo_bit")]
                    crate::util::metadata::vo_bit::bzero_vo_bit(cell, cell_size);

                    intervals.add(cell);
                    cell += cell_size;
                    debug_assert_eq!(cursor, cell);
                }
            }
        }

        self.store_free_list(intervals.finish());
    }

    /// Get the chunk containing the block.
    pub fn chunk(&self) -> Chunk {
        Chunk::from_unaligned_address(self.start())
    }

    /// Initialize a clean block after acquired from page-resource.
    pub fn init(&self) {
        self.set_state(BlockState::Unmarked);
    }

    /// Deinitalize a block before releasing.
    pub fn deinit(&self) {
        self.set_state(BlockState::Unallocated);
    }
}

/// The block allocation state.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BlockState {
    /// the block is not allocated.
    Unallocated,
    /// the block is allocated but not marked.
    Unmarked,
    /// the block is allocated and marked.
    Marked,
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
    fn from(state: u8) -> Self {
        match state {
            Self::MARK_UNALLOCATED => BlockState::Unallocated,
            Self::MARK_UNMARKED => BlockState::Unmarked,
            Self::MARK_MARKED => BlockState::Marked,
            _ => unreachable!(),
        }
    }
}

impl From<BlockState> for u8 {
    fn from(state: BlockState) -> Self {
        match state {
            BlockState::Unallocated => BlockState::MARK_UNALLOCATED,
            BlockState::Unmarked => BlockState::MARK_UNMARKED,
            BlockState::Marked => BlockState::MARK_MARKED,
        }
    }
}

/// Tag bit in the first word of an interval: the interval is a single cell, and it has no end word.
const INTERVAL_SINGLE_CELL: usize = 1;

/// Write the header of the interval `[start, end)`: its first word is the address of the next interval,
/// with [`INTERVAL_SINGLE_CELL`] set if the interval is one cell, and otherwise its second word is `end`.
fn write_interval(start: Address, end: Address, cell_size: usize, next: Address) {
    unsafe {
        if end == start + cell_size {
            start.store::<usize>(next.as_usize() | INTERVAL_SINGLE_CELL);
        } else {
            start.store::<Address>(next);
            (start + BYTES_IN_ADDRESS).store::<Address>(end);
        }
    }
}

/// Read the header of the interval starting at `start`, and return its end and the next interval.
fn read_interval(start: Address, cell_size: usize) -> (Address, Address) {
    let word = unsafe { start.load::<usize>() };
    let next = unsafe { Address::from_usize(word & !INTERVAL_SINGLE_CELL) };
    let end = if word & INTERVAL_SINGLE_CELL != 0 {
        start + cell_size
    } else {
        unsafe { (start + BYTES_IN_ADDRESS).load::<Address>() }
    };
    (end, next)
}

/// Builds a block's free list from free cells visited in address order.
///
/// When `coalesce` is set, adjacent free cells are merged into intervals and the list is in
/// address order. Otherwise every cell is an interval of its own and is pushed to the front of
/// the list, so the list is a canonical free list of cells, highest address first.
pub(crate) struct IntervalListBuilder {
    cell_size: usize,
    coalesce: bool,
    head: Address,
    /// The last interval that was written, whose next field is patched when another interval follows.
    last: Address,
    /// The interval being extended, `[interval_start, interval_end)`; empty when `interval_start` is zero.
    interval_start: Address,
    interval_end: Address,
}

impl IntervalListBuilder {
    pub(crate) fn new(cell_size: usize, coalesce: bool) -> Self {
        Self {
            cell_size,
            coalesce,
            head: Address::ZERO,
            last: Address::ZERO,
            interval_start: Address::ZERO,
            interval_end: Address::ZERO,
        }
    }

    /// Add the free cell at `cell`. Cells must be added in increasing address order.
    pub(crate) fn add(&mut self, cell: Address) {
        if !self.coalesce {
            write_interval(cell, cell + self.cell_size, self.cell_size, self.head);
            self.head = cell;
            return;
        }
        if !self.interval_start.is_zero() && cell == self.interval_end {
            self.interval_end = cell + self.cell_size;
            return;
        }
        self.close_interval();
        self.interval_start = cell;
        self.interval_end = cell + self.cell_size;
    }

    /// Add the free interval `[start, end)`, whose length is a multiple of the cell size.
    pub(crate) fn add_interval(&mut self, start: Address, end: Address) {
        debug_assert_eq!((end - start) % self.cell_size, 0);
        if !self.coalesce {
            let mut cell = start;
            while cell < end {
                self.add(cell);
                cell += self.cell_size;
            }
            return;
        }
        self.add(start);
        self.interval_end = end;
    }

    fn close_interval(&mut self) {
        if self.interval_start.is_zero() {
            return;
        }
        write_interval(
            self.interval_start,
            self.interval_end,
            self.cell_size,
            Address::ZERO,
        );
        if self.last.is_zero() {
            self.head = self.interval_start;
        } else {
            let tag = unsafe { self.last.load::<usize>() } & INTERVAL_SINGLE_CELL;
            unsafe {
                self.last
                    .store::<usize>(self.interval_start.as_usize() | tag)
            };
        }
        self.last = self.interval_start;
        self.interval_start = Address::ZERO;
    }

    /// Finish and return the head of the list (zero if no cell was added).
    pub(crate) fn finish(mut self) -> Address {
        self.close_interval();
        self.head
    }
}
