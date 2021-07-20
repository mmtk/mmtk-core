use super::chunk::Chunk;
use super::line::Line;
use crate::util::constants::*;
use crate::util::metadata::side_metadata::{self, *};
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use std::{
    iter::Step,
    ops::Range,
    sync::{
        atomic::{AtomicPtr, AtomicU8, AtomicUsize, Ordering},
        Mutex, MutexGuard,
    },
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
            LOCAL_SIDE_METADATA_BASE_ADDRESS.as_usize()
        } else {
            Line::MARK_TABLE.accumulated_size()
        },
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    /// Block mark table (side)
    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: Self::DEFRAG_STATE_TABLE.accumulated_size(),
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
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES) }
    }

    /// Get the chunk containing the block.
    #[inline(always)]
    pub fn chunk(&self) -> Chunk {
        Chunk::from(Chunk::align(self.0))
    }

    /// Get the address range of the block's line mark table.
    #[allow(clippy::assertions_on_constants)]
    #[inline(always)]
    pub fn line_mark_table(&self) -> Range<Address> {
        debug_assert!(!super::BLOCK_ONLY);
        let start = side_metadata::address_to_meta_address(&Line::MARK_TABLE, self.start());
        let end = start + Block::LINES;
        start..end
    }

    const MARK_UNALLOCATED: u8 = 0;
    const MARK_UNMARKED: u8 = u8::MAX;
    const MARK_MARKED: u8 = u8::MAX - 1;

    #[inline(always)]
    fn mark_byte(&self) -> &AtomicU8 {
        unsafe {
            &*side_metadata::address_to_meta_address(&Self::MARK_TABLE, self.start())
                .to_mut_ptr::<AtomicU8>()
        }
    }

    /// Get block mark state.
    #[inline(always)]
    pub fn get_state(&self) -> BlockState {
        match self.mark_byte().load(Ordering::Acquire) {
            Self::MARK_UNALLOCATED => BlockState::Unallocated,
            Self::MARK_UNMARKED => BlockState::Unmarked,
            Self::MARK_MARKED => BlockState::Marked,
            unavailable_lines => BlockState::Reusable { unavailable_lines },
        }
    }

    /// Set block mark state.
    #[inline(always)]
    pub fn set_state(&self, state: BlockState) {
        let v = match state {
            BlockState::Unallocated => Self::MARK_UNALLOCATED,
            BlockState::Unmarked => Self::MARK_UNMARKED,
            BlockState::Marked => Self::MARK_MARKED,
            BlockState::Reusable { unavailable_lines } => unavailable_lines,
        };
        self.mark_byte().store(v, Ordering::Release)
    }

    // Defrag byte

    const DEFRAG_SOURCE_STATE: u8 = u8::MAX;

    #[inline(always)]
    fn defrag_byte(&self) -> &AtomicU8 {
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
        Some(Self::from(start.start() + (count << Self::LOG_BYTES)))
    }
    /// result = block_address - count * block_size
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Self::from(start.start() - (count << Self::LOG_BYTES)))
    }
}

struct Node<T> {
    value: T,
    next: AtomicPtr<Node<T>>,
}

/// A non-block single-linked list to store blocks.
#[derive(Default)]
pub struct BlockList {
    head: AtomicPtr<Node<Block>>,
    len: AtomicUsize,
    sync: Mutex<()>,
}

impl BlockList {
    /// Get number of blocks in this list.
    #[inline]
    pub fn len(&self) -> usize {
        self.len.load(Ordering::SeqCst)
    }

    /// Add a block to the list.
    #[inline]
    pub fn push(&self, block: Block) {
        let node = Box::leak(box Node {
            value: block,
            next: AtomicPtr::default(),
        });
        loop {
            let next = self.head.load(Ordering::SeqCst);
            node.next.store(next, Ordering::SeqCst);
            if self
                .head
                .compare_exchange(next, node, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                break;
            }
        }
        self.len.fetch_add(1, Ordering::SeqCst);
    }

    /// Pop a block out of the list.
    #[inline]
    pub fn pop(&self) -> Option<Block> {
        loop {
            let head = self.head.load(Ordering::SeqCst);
            if head.is_null() {
                return None;
            }
            let next = unsafe { (*head).next.load(Ordering::SeqCst) };
            if self
                .head
                .compare_exchange(head, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let block = unsafe { (*head).value };
                unsafe { Box::from_raw(head) };
                self.len.fetch_sub(1, Ordering::SeqCst);
                return Some(block);
            }
        }
    }

    /// Clear the list.
    #[inline]
    pub fn reset(&self) {
        let _guard = self.sync.lock().unwrap();
        self.len.store(0, Ordering::SeqCst);
        loop {
            let head = self.head.load(Ordering::SeqCst);
            if head.is_null() {
                return;
            }
            self.head.store(
                unsafe { (*head).next.load(Ordering::SeqCst) },
                Ordering::SeqCst,
            );
            unsafe { Box::from_raw(head) };
        }
    }

    /// Iterate all blocks in the list.
    /// Note: this can only be called by a single thread.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = Block> {
        BlockListIter {
            head: self.head.load(Ordering::SeqCst),
        }
    }
}

impl<'a> IntoIterator for &'a BlockList {
    type Item = Block;
    type IntoIter = impl Iterator<Item = Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

struct BlockListIter {
    head: *mut Node<Block>,
}

impl Iterator for BlockListIter {
    type Item = Block;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.head.is_null() {
            None
        } else {
            let node = unsafe { &mut *self.head };
            self.head = node.next.load(Ordering::SeqCst);
            Some(node.value)
        }
    }
}

struct DrainFilter<'a, F: 'a + FnMut(&mut Block) -> bool> {
    head: &'a AtomicPtr<Node<Block>>,
    predicate: F,
    _guard: MutexGuard<'a, ()>,
}

impl<'a, F: 'a + FnMut(&mut Block) -> bool> Iterator for DrainFilter<'a, F> {
    type Item = Block;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let node_ptr = self.head.load(Ordering::SeqCst);
            if node_ptr.is_null() {
                return None;
            } else {
                let node = unsafe { &mut *node_ptr };
                if (self.predicate)(&mut node.value) {
                    let block = node.value;
                    self.head
                        .store(node.next.load(Ordering::SeqCst), Ordering::SeqCst);
                    unsafe { Box::from_raw(node_ptr) };
                    return Some(block);
                } else {
                    self.head = &node.next;
                }
            }
        }
    }
}
