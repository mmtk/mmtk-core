// all from Wenyu's Immix

use std::iter::Step;

use atomic::Ordering;

use crate::{util::{Address, OpaquePointer, alloc::free_list_allocator::{self, BYTES_IN_BLOCK, FreeListAllocator, LOG_BYTES_IN_BLOCK}, metadata::{
            side_metadata::{
                self, SideMetadataOffset, SideMetadataSpec, LOCAL_SIDE_METADATA_BASE_OFFSET,
            },
            store_metadata, MetadataSpec,
        }}, vm::VMBinding};

use super::{MARKSWEEP_LOCAL_SIDE_METADATA_BASE_OFFSET, MarkSweepSpace, chunks::Chunk};

#[derive(Debug, Clone, Copy, PartialOrd, PartialEq)]
pub struct Block(Address);

impl Block {
    /// Align the address to a block boundary.
    pub const fn align(address: Address) -> Address {
        address.align_down(BYTES_IN_BLOCK)
    }

    /// Block mark table (side)
    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: MARKSWEEP_LOCAL_SIDE_METADATA_BASE_OFFSET,
        log_num_of_bits: 3,
        log_min_obj_size: free_list_allocator::LOG_BYTES_IN_BLOCK,
    };

    pub const NEXT_BLOCK_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::MARK_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };

    pub const PREV_BLOCK_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::NEXT_BLOCK_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };

    pub const FREE_LIST_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::PREV_BLOCK_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };
    pub const SIZE_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::FREE_LIST_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };
    pub const LOCAL_FREE_LIST_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::SIZE_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };
    pub const THREAD_FREE_LIST_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::LOCAL_FREE_LIST_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };

    pub const BLOCK_LIST_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::THREAD_FREE_LIST_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };

    pub const TLS_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::BLOCK_LIST_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };

    /// Get block start address
    pub const fn start(&self) -> Address {
        self.0
    }

    /// Get block mark state.
    #[inline(always)]
    pub fn get_state(&self) -> BlockState {
        let byte =
            side_metadata::load_atomic(&Self::MARK_TABLE, self.start(), Ordering::SeqCst) as u8;
        byte.into()
    }

    /// Set block mark state.
    #[inline(always)]
    pub fn set_state(&self, state: BlockState) {
        let state = u8::from(state) as usize;
        side_metadata::store_atomic(&Self::MARK_TABLE, self.start(), state, Ordering::SeqCst);
    }

    /// Sweep this block.
    /// Return true if the block is swept.
    #[inline(always)]
    pub fn sweep<VM: VMBinding>(&self, space: &MarkSweepSpace<VM>) -> bool {
        match self.get_state() {
            BlockState::Unallocated => false,
            BlockState::Unmarked => {
                let prev = FreeListAllocator::<VM>::load_prev_block(self.0);
                let next = FreeListAllocator::<VM>::load_next_block(self.0);
                if next.is_zero() {
                    let mut block_list = FreeListAllocator::<VM>::load_block_list(self.0);
                    block_list.last = prev;
                } else {
                    FreeListAllocator::<VM>::store_prev_block(next, prev);
                }
                if prev.is_zero() {
                    let mut block_list = FreeListAllocator::<VM>::load_block_list(self.0);
                    block_list.first = next;
                } else {
                    FreeListAllocator::<VM>::store_next_block(prev, next);
                }
                space.release_block(self.0);
                true
            }
            BlockState::Marked => {
                // The block is live.
                false
            }
            _ => unreachable!(),
        }
    }

    /// Get the chunk containing the block.
    #[inline(always)]
    pub fn chunk(&self) -> Chunk {
        Chunk::from(Chunk::align(self.0))
    }

    /// Initialize a clean block after acquired from page-resource.
    #[inline]
    pub fn init(&self) {
        self.set_state( BlockState::Unmarked);
    }

    /// Deinitalize a block before releasing.
    #[inline]
    pub fn deinit(&self) {
        self.set_state(BlockState::Unallocated);
    }

    /// Get the block from a given address.
    /// The address must be block-aligned.
    #[inline(always)]
    pub fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(BYTES_IN_BLOCK));
        Self(address)
    }
}

unsafe impl Step for Block {
    /// Get the number of blocks between the given two blocks.
    #[inline(always)]
    #[allow(clippy::assertions_on_constants)]
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        if start > end {
            return None;
        }
        Some((end.start() - start.start()) >> LOG_BYTES_IN_BLOCK)
    }
    /// result = block_address + count * block_size
    #[inline(always)]
    fn forward(start: Self, count: usize) -> Self {
        Self::from(start.start() + (count << LOG_BYTES_IN_BLOCK))
    }
    /// result = block_address + count * block_size
    #[inline(always)]
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        if start.start().as_usize() > usize::MAX - (count << LOG_BYTES_IN_BLOCK) {
            return None;
        }
        Some(Self::forward(start, count))
    }
    /// result = block_address + count * block_size
    #[inline(always)]
    fn backward(start: Self, count: usize) -> Self {
        Self::from(start.start() - (count << LOG_BYTES_IN_BLOCK))
    }
    /// result = block_address - count * block_size
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        if start.start().as_usize() < (count << LOG_BYTES_IN_BLOCK) {
            return None;
        }
        Some(Self::backward(start, count))
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
    /// the block is marked as reusable.
    UnmarkedAcknowledged,
}

impl BlockState {
    /// Private constant
    const MARK_UNALLOCATED: u8 = 0;
    /// Private constant
    const MARK_UNMARKED: u8 = u8::MAX;
    /// Private constant
    const MARK_MARKED: u8 = u8::MAX - 1;
    // Private constant
    const MARK_UNMARKED_ACKNOWLEDGED: u8 = u8::MAX - 2;
}

impl From<u8> for BlockState {
    #[inline(always)]
    fn from(state: u8) -> Self {
        match state {
            Self::MARK_UNALLOCATED => BlockState::Unallocated,
            Self::MARK_UNMARKED => BlockState::Unmarked,
            Self::MARK_MARKED => BlockState::Marked,
            Self::MARK_UNMARKED_ACKNOWLEDGED => BlockState::UnmarkedAcknowledged,
            _ => unreachable!()
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
            BlockState::UnmarkedAcknowledged => BlockState::MARK_UNMARKED_ACKNOWLEDGED,
        }
    }
}
