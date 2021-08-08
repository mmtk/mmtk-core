// all from Wenyu's Immix

use std::iter::Step;

use atomic::Ordering;

use crate::{
    util::{
        alloc::free_list_allocator::{self, BYTES_IN_BLOCK, LOG_BYTES_IN_BLOCK},
        metadata::{
            side_metadata::{
                self, SideMetadataOffset, SideMetadataSpec, LOCAL_SIDE_METADATA_BASE_OFFSET,
            },
            store_metadata, MetadataSpec,
        },
        Address, OpaquePointer,
    },
    vm::VMBinding,
};

use super::{metadata::ALLOC_SIDE_METADATA_SPEC, MarkSweepSpace};
use crate::{util::{Address, OpaquePointer, alloc::{free_list_allocator::{self, BYTES_IN_BLOCK, LOG_BYTES_IN_BLOCK}}, metadata::side_metadata::{self, LOCAL_SIDE_METADATA_BASE_OFFSET, SideMetadataOffset, SideMetadataSpec}}, vm::VMBinding};

use super::{MarkSweepSpace, metadata::ALLOC_SIDE_METADATA_SPEC};

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
        offset: SideMetadataOffset::layout_after(&ALLOC_SIDE_METADATA_SPEC),
        log_num_of_bits: 3,
        log_min_obj_size: free_list_allocator::LOG_BYTES_IN_BLOCK,
    };

    pub const NEXT_BLOCK_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::MARK_TABLE),
        log_num_of_bits: 6,
        log_min_obj_size: 16,
    };

    pub const FREE_LIST_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::NEXT_BLOCK_TABLE),
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
    pub const TLS_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: SideMetadataOffset::layout_after(&Block::THREAD_FREE_LIST_TABLE),
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
                // Release the block if it is allocated but not marked by the current GC.
                space.release_block(self.0);
                true
            }
            BlockState::Marked => {
                // The block is live.
                // let tls = space.load_block_tls(self.0);
                // let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(tls) };
                // eprintln!("block level sweep");
                // let mut marked_blocks = space.marked_blocks.lock().unwrap();
                // let blocks = marked_blocks.get_mut(&tls);
                // match blocks {
                //     Some(blocks) => {
                //         let size = space.load_block_cell_size(self.0);
                //         let bin = crate::util::alloc::FreeListAllocator::<VM>::mi_bin(size);
                //         let block_queue = blocks.get_mut(bin as usize).unwrap();
                //         store_metadata::<VM>(
                //             &MetadataSpec::OnSide(space.get_next_metadata_spec()),
                //             unsafe { self.0.to_object_reference() },
                //             block_queue.first.as_usize(),
                //             None,
                //             None,
                //         );
                //         block_queue.first = self.0;
                //     }
                //     None => {
                //         marked_blocks.insert(tls, free_list_allocator::BLOCK_LISTS_EMPTY.to_vec());
                //     }
                // }
                false
            }
            _ => unreachable!(),
        }
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
