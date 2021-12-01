// all from Wenyu's Immix

use std::iter::Step;

use atomic::Ordering;

use crate::{util::{Address, OpaquePointer, alloc::free_list_allocator::{self, BYTES_IN_BLOCK, BlockList, FreeListAllocator, LOG_BYTES_IN_BLOCK}, metadata::{MetadataSpec, load_metadata, side_metadata::{
                self, SideMetadataOffset, SideMetadataSpec, LOCAL_SIDE_METADATA_BASE_OFFSET,
            }, store_metadata}, VMThread}, vm::VMBinding};

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

    #[inline]
    pub fn load_free_list<VM: VMBinding>(&self) -> Address {
        unsafe {
            Address::from_usize(load_metadata::<VM>(
                &MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
                self.0.to_object_reference(),
                None,
                None,
            ))
        }
    }

    #[inline]
    pub fn store_free_list<VM: VMBinding>(&self, free_list: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::FREE_LIST_TABLE),
            unsafe { self.0.to_object_reference() },
            free_list.as_usize(),
            None,
            None,
        );
    }

    #[inline]
    pub fn load_local_free_list<VM: VMBinding>(&self) -> Address {
        unsafe {
            Address::from_usize(load_metadata::<VM>(
                &MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
                self.0.to_object_reference(),
                None,
                None,
            ))
        }
    }

    #[inline]
    pub fn store_local_free_list<VM: VMBinding>(&self, local_free: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
            unsafe { self.0.to_object_reference() },
            local_free.as_usize(),
            None,
            None,
        );
    }

    #[inline]
    pub fn load_thread_free_list<VM: VMBinding>(&self) -> Address {
        unsafe {
            Address::from_usize(load_metadata::<VM>(
                &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
                self.0.to_object_reference(),
                None,
                Some(Ordering::SeqCst),
            ))
        }
    }

    #[inline]
    pub fn store_thread_free_list<VM: VMBinding>(&self, thread_free: Address) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
            unsafe { self.0.to_object_reference() },
            thread_free.as_usize(),
            None,
            None,
        );
    }

    // #[inline]
    // pub fn cas_thread_free_list(
    //     &self,
    //     block: Address,
    //     old_thread_free: Address,
    //     new_thread_free: Address,
    // ) -> bool {
    //     compare_exchange_metadata::<VM>(
    //         &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
    //         unsafe { block.to_object_reference() },
    //         old_thread_free.as_usize(),
    //         new_thread_free.as_usize(),
    //         None,
    //         Ordering::SeqCst,
    //         Ordering::SeqCst,
    //     )
    // }

    pub fn load_prev_block<VM: VMBinding>(&self) -> Block {
        assert!(!self.0.is_zero());
        let prev = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::PREV_BLOCK_TABLE),
            unsafe { self.0.to_object_reference() },
            None,
            None,
        );
        Block::from(unsafe { Address::from_usize(prev) })
    }

    pub fn load_next_block<VM: VMBinding>(&self) -> Block {
        assert!(!self.is_zero());
        let next = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::NEXT_BLOCK_TABLE),
            unsafe { self.0.to_object_reference() },
            None,
            None,
        );
        Block::from(unsafe { Address::from_usize(next) })
    }

    pub fn store_next_block<VM: VMBinding>(&self, next: Block) {
        assert!(!self.0.is_zero());
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::NEXT_BLOCK_TABLE),
            unsafe { self.0.to_object_reference() },
            next.start().as_usize(),
            None,
            None,
        );
        // eprintln!("store {} -> {}", self.start(), next.start());
    }

    pub fn store_prev_block<VM: VMBinding>(&self, prev: Block) {
        assert!(!self.0.is_zero());
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::PREV_BLOCK_TABLE),
            unsafe { self.0.to_object_reference() },
            prev.start().as_usize(),
            None,
            None,
        );
        // eprintln!("store {} -> {}", prev.start(), self.start());
    }

    pub fn store_block_list<VM: VMBinding>(&self, block_list: &BlockList) {
        assert!(!self.0.is_zero());
        assert!(self.load_prev_block::<VM>().is_zero() || self.load_next_block::<VM>().is_zero());
        assert!(block_list.first == *self || block_list.last == *self);
        // let ptr: *mut BlockList = &mut block_list;
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::BLOCK_LIST_TABLE),
            unsafe { self.0.to_object_reference() },
            unsafe { std::mem::transmute::<&BlockList, usize>(block_list) },
            None,
            None,
        );
        let loaded = self.load_block_list::<VM>();
        unsafe{assert!((*loaded).first == block_list.first);}
    }

    pub fn load_block_list<VM: VMBinding>(&self) -> *mut BlockList {
        assert!(!self.0.is_zero());
        let block_list = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::BLOCK_LIST_TABLE),
            unsafe { self.0.to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        );
        let ptr = unsafe { std::mem::transmute::<usize, *mut BlockList>(block_list) };
        ptr
    }

    pub fn load_block_cell_size<VM: VMBinding>(&self) -> usize {
        load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::SIZE_TABLE),
            unsafe { self.0.to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        )
    }
    
    pub fn store_block_cell_size<VM: VMBinding>(&self, size: usize) {
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::SIZE_TABLE),
            unsafe { self.0.to_object_reference() },
            size,
            None,
            None,
        );
    }

    
    pub fn store_tls<VM: VMBinding>(&self, tls: VMThread) {
        let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(tls.0) };
        store_metadata::<VM>(
            &MetadataSpec::OnSide(Block::TLS_TABLE),
            unsafe { self.start().to_object_reference() },
            tls,
            None,
            None,
        );
    }

    
    pub fn load_tls<VM: VMBinding>(&self) -> OpaquePointer {
        let tls = load_metadata::<VM>(
            &MetadataSpec::OnSide(Block::TLS_TABLE),
            unsafe { self.start().to_object_reference() },
            None,
            Some(Ordering::SeqCst),
        );
        unsafe { std::mem::transmute::<usize, OpaquePointer>(tls) }
    }

    pub fn is_zero(&self) -> bool {
        self.start().is_zero()
    }

    pub fn has_free_cells<VM: VMBinding>(&self) -> bool {
        debug_assert!(!self.is_zero());
        !self.load_free_list::<VM>().is_zero()
    }

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
    pub fn sweep<VM: VMBinding>(self, space: &MarkSweepSpace<VM>) -> bool {
        match self.get_state() {
            BlockState::Unallocated => false,
            BlockState::Unmarked => {
                // // eprintln!("block {} is unmarked", self.0);
                let prev = self.load_prev_block::<VM>();
                let next = self.load_next_block::<VM>();
                // eprintln!("sweep: {} -> {} -> {}", prev.0, self.0, next.0);
                if next.is_zero() || prev.is_zero() {
                    unsafe { 
                        let mut block_list = self.load_block_list::<VM>();
                        // eprintln!("sweep: {} -> {} -> {}, {:?}, first = {}, last = {}", prev.0, self.0, next.0, block_list, (*block_list).first.start(), (*block_list).last.start());
                        (*block_list).remove::<VM>(self);
                    }
                } else {
                    // // eprintln!("block: store {} -> {}", prev, next); 
                    next.store_prev_block::<VM>(prev);
                    prev.store_next_block::<VM>(next);
                }
                // check what list
                // try to remove copy
                space.release_block(self);
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
    pub const fn from(address: Address) -> Self {
        // debug_assert!(address.is_aligned_to(BYTES_IN_BLOCK));
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
