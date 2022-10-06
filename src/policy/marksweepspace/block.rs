// adapted from Immix

use atomic::Ordering;

use super::MarkSweepSpace;
use crate::util::heap::chunk_map::*;
use crate::util::linear_scan::Region;
use crate::{
    util::{
        alloc::free_list_allocator::BlockList, metadata::side_metadata::SideMetadataSpec, Address,
        OpaquePointer, VMThread,
    },
    vm::VMBinding,
};

#[derive(Debug, Clone, Copy, PartialOrd, PartialEq)]
#[repr(C)]
pub struct Block(Address);

impl From<Address> for Block {
    #[inline(always)]
    fn from(address: Address) -> Block {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }
}

impl From<Block> for Address {
    #[inline(always)]
    fn from(block: Block) -> Address {
        block.0
    }
}

impl Region for Block {
    const LOG_BYTES: usize = 16;
}

impl Block {
    pub const ZERO_BLOCK: Self = Self(Address::ZERO);

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
    // pub const LOCAL_FREE_LIST_TABLE: SideMetadataSpec =
    //     crate::util::metadata::side_metadata::spec_defs::MS_LOCAL_FREE;

    // pub const THREAD_FREE_LIST_TABLE: SideMetadataSpec =
    //     crate::util::metadata::side_metadata::spec_defs::MS_THREAD_FREE;

    pub const SIZE_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_SIZE;

    pub const BLOCK_LIST_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_LIST;

    pub const TLS_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::MS_BLOCK_TLS;

    #[inline]
    pub fn load_free_list(&self) -> Address {
        unsafe { Address::from_usize(Block::FREE_LIST_TABLE.load::<usize>(self.0)) }
    }

    #[inline]
    pub fn store_free_list(&self, free_list: Address) {
        unsafe { Block::FREE_LIST_TABLE.store::<usize>(self.0, free_list.as_usize()) }
    }

    // #[inline]
    // pub fn load_local_free_list<VM: VMBinding>(&self) -> Address {
    //     unsafe {
    //         Address::from_usize(load_metadata::<VM>(
    //             &MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
    //             self.0.to_object_reference(),
    //             None,
    //             None,
    //         ))
    //     }
    // }

    // #[inline]
    // pub fn store_local_free_list<VM: VMBinding>(&self, local_free: Address) {
    //     store_metadata::<VM>(
    //         &MetadataSpec::OnSide(Block::LOCAL_FREE_LIST_TABLE),
    //         unsafe { self.0.to_object_reference() },
    //         local_free.as_usize(),
    //         None,
    //         None,
    //     );
    // }

    // #[inline]
    // pub fn load_thread_free_list<VM: VMBinding>(&self) -> Address {
    //     unsafe {
    //         Address::from_usize(load_metadata::<VM>(
    //             &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
    //             self.0.to_object_reference(),
    //             None,
    //             Some(Ordering::SeqCst),
    //         ))
    //     }
    // }

    // #[inline]
    // pub fn store_thread_free_list<VM: VMBinding>(&self, thread_free: Address) {
    //     store_metadata::<VM>(
    //         &MetadataSpec::OnSide(Block::THREAD_FREE_LIST_TABLE),
    //         unsafe { self.0.to_object_reference() },
    //         thread_free.as_usize(),
    //         None,
    //         None,
    //     );
    // }

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

    pub fn load_prev_block(&self) -> Block {
        debug_assert!(!self.0.is_zero());
        let prev = unsafe { Address::from_usize(Block::PREV_BLOCK_TABLE.load::<usize>(self.0)) };
        Block::from(prev)
    }

    pub fn load_next_block(&self) -> Block {
        debug_assert!(!self.is_zero());
        let next = unsafe { Address::from_usize(Block::NEXT_BLOCK_TABLE.load::<usize>(self.0)) };
        Block::from(next)
    }

    pub fn store_next_block(&self, next: Block) {
        debug_assert!(!self.0.is_zero());
        unsafe {
            Block::NEXT_BLOCK_TABLE.store::<usize>(self.0, next.start().as_usize());
        }
    }

    pub fn store_prev_block(&self, prev: Block) {
        debug_assert!(!self.0.is_zero());
        unsafe {
            Block::PREV_BLOCK_TABLE.store::<usize>(self.0, prev.start().as_usize());
        }
    }

    pub fn store_block_list(&self, block_list: &BlockList) {
        debug_assert!(!self.0.is_zero());
        let block_list_usize: usize =
            unsafe { std::mem::transmute::<&BlockList, usize>(block_list) };
        unsafe {
            Block::BLOCK_LIST_TABLE.store::<usize>(self.0, block_list_usize);
        }
    }

    pub fn load_block_list(&self) -> *mut BlockList {
        debug_assert!(!self.0.is_zero());
        let block_list = Block::BLOCK_LIST_TABLE.load_atomic::<usize>(self.0, Ordering::SeqCst);
        unsafe { std::mem::transmute::<usize, *mut BlockList>(block_list) }
    }

    pub fn load_block_cell_size(&self) -> usize {
        Block::SIZE_TABLE.load_atomic::<usize>(self.0, Ordering::SeqCst)
    }

    pub fn store_block_cell_size(&self, size: usize) {
        unsafe { Block::SIZE_TABLE.store::<usize>(self.0, size) }
    }

    pub fn store_tls(&self, tls: VMThread) {
        let tls = unsafe { std::mem::transmute::<OpaquePointer, usize>(tls.0) };
        unsafe { Block::TLS_TABLE.store(self.start(), tls) }
    }

    pub fn load_tls(&self) -> VMThread {
        let tls = Block::TLS_TABLE.load_atomic::<usize>(self.start(), Ordering::SeqCst);
        VMThread(OpaquePointer::from_address(unsafe {
            Address::from_usize(tls)
        }))
    }

    pub fn is_zero(&self) -> bool {
        self.start().is_zero()
    }

    pub fn has_free_cells(&self) -> bool {
        debug_assert!(!self.is_zero());
        !self.load_free_list().is_zero()
    }

    /// Get block start address
    pub const fn start(&self) -> Address {
        self.0
    }

    /// Get block mark state.
    #[inline(always)]
    pub fn get_state(&self) -> BlockState {
        let byte = Self::MARK_TABLE.load_atomic::<u8>(self.start(), Ordering::SeqCst);
        byte.into()
    }

    /// Set block mark state.
    #[inline(always)]
    pub fn set_state(&self, state: BlockState) {
        let state = u8::from(state);
        Self::MARK_TABLE.store_atomic::<u8>(self.start(), state, Ordering::SeqCst);
    }

    /// Release this block if it is unmarked. Return true if the block is release.
    #[inline(always)]
    pub fn attempt_release<VM: VMBinding>(self, space: &MarkSweepSpace<VM>) -> bool {
        match self.get_state() {
            BlockState::Unallocated => false,
            BlockState::Unmarked => {
                unsafe {
                    let block_list = loop {
                        let list = self.load_block_list();
                        (*list).lock();
                        if list == self.load_block_list() {
                            break list;
                        }
                        (*list).unlock();
                    };
                    (*block_list).remove(self);
                    (*block_list).unlock();
                }
                space.release_block(self);
                true
            }
            BlockState::Marked => {
                // The block is live.
                false
            }
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
        self.set_state(BlockState::Unmarked);
    }

    /// Deinitalize a block before releasing.
    #[inline]
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
    #[inline(always)]
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
    #[inline(always)]
    fn from(state: BlockState) -> Self {
        match state {
            BlockState::Unallocated => BlockState::MARK_UNALLOCATED,
            BlockState::Unmarked => BlockState::MARK_UNMARKED,
            BlockState::Marked => BlockState::MARK_MARKED,
        }
    }
}
