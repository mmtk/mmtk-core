use crate::util::{Address, ObjectReference};
use crate::util::side_metadata::{self, *};
use crate::util::constants::*;
use std::{iter::Step, ops::Range, sync::{Mutex, MutexGuard, atomic::{AtomicPtr, AtomicUsize, Ordering}}};
use super::line::Line;
use super::chunk::Chunk;
use crate::vm::*;



#[repr(u8)]
#[derive(Debug, PartialEq)]
pub enum BlockState {
    Unallocated = 0,
    Unmarked = 1,
    Marked = 2,
    Reusable = 3,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq)]
pub struct Block(Address);

impl Block {
    pub const LOG_BYTES: usize = 15;

    pub const BYTES: usize = 1 << Self::LOG_BYTES;
    pub const LOG_PAGES: usize = Self::LOG_BYTES - LOG_BYTES_IN_PAGE as usize;
    pub const PAGES: usize = 1 << Self::LOG_PAGES;

    pub const LOG_LINES: usize = Self::LOG_BYTES - Line::LOG_BYTES;
    pub const LINES: usize = 1 << Self::LOG_LINES;

    pub const DEFRAG_STATE_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: if super::BLOCK_ONLY { 0 } else { Line::MARK_TABLE.accumulated_size() },
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: Self::DEFRAG_STATE_TABLE.accumulated_size(),
        log_num_of_bits: 3,
        log_min_obj_size: Self::LOG_BYTES,
    };

    pub const fn align(address: Address) -> Address {
        address.align_down(Self::BYTES)
    }

    pub const fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    #[inline(always)]
    pub fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self(VM::VMObjectModel::ref_to_address(object).align_down(Self::BYTES))
    }

    pub const fn start(&self) -> Address {
        self.0
    }

    pub const fn end(&self) -> Address {
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES) }
    }

    #[inline(always)]
    fn mark_byte(&self) -> u8 {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.start()) as u8 }
    }

    #[inline(always)]
    fn set_mark_byte(&self, byte: u8) {
        unsafe { side_metadata::store(Self::MARK_TABLE, self.start(), byte as usize) }
    }

    pub const fn chunk(&self) -> Chunk {
        Chunk::from(Chunk::align(self.0))
    }

    pub const fn line_mark_table(&self) -> Range<Address> {
        debug_assert!(!super::BLOCK_ONLY);
        let start = side_metadata::address_to_meta_address(Line::MARK_TABLE, self.start());
        let end = start + Block::LINES;
        start..end
    }

    #[inline]
    pub fn get_state(&self) -> BlockState {
        unsafe {
            std::mem::transmute(self.mark_byte())
        }
    }

    #[inline]
    pub fn set_state(&self, state: BlockState) {
        if cfg!(debug_assertions) {
            if state == BlockState::Marked || state == BlockState::Reusable {
                assert!(!self.is_defrag_source(), "{:?}", self)
            }
        }
        self.set_mark_byte(state as _)
    }

    #[inline]
    pub fn is_defrag_source(&self) -> bool {
        unsafe {
            side_metadata::load(Self::DEFRAG_STATE_TABLE, self.start()) as u8 == 1
        }
    }

    #[inline]
    pub fn set_as_defrag_source(&self) {
        unsafe {
            side_metadata::store(Self::DEFRAG_STATE_TABLE, self.start(), 1)
        }
    }

    #[inline]
    pub fn init(&self) {
        self.set_state(BlockState::Marked);
        unsafe {
            side_metadata::store(Self::DEFRAG_STATE_TABLE, self.start(), 0)
        }
    }

    #[inline]
    pub fn deinit(&self) {
        self.set_state(BlockState::Unallocated);
        unsafe {
            side_metadata::store(Self::DEFRAG_STATE_TABLE, self.start(), 0)
        }
    }

    // #[inline]
    // pub fn set_marked_lines(&self, value: u8) {
    //     self.set_mark_byte(value + 4)
    // }

    // #[inline]
    // pub fn get_marked_lines(&self) -> u8 {
    //     self.set_mark_byte(value + 4)
    // }

    // #[inline]
    // pub fn is_defrag(&self) -> bool {
    //     self.mark_byte() == BlockMarkState::Defrag as _
    // }

    // #[inline]
    // pub fn clear_mark(&self) {
    //     self.set_mark_byte(BlockMarkState::Unmarked as _)
    // }

    pub const fn lines(&self) -> Range<Line> {
        debug_assert!(!super::BLOCK_ONLY);
        Line::from(self.start()) .. Line::from(self.end())
    }

    #[inline]
    pub fn count_holes(&self, line_mark_state: u8) -> usize {
        let mut holes = 0;
        let mut prev_line_is_marked = true;
        for line in self.lines() {
            if !line.is_marked(line_mark_state) {
                if prev_line_is_marked {
                    holes += 1;
                }
                prev_line_is_marked = false;
            } else {
                prev_line_is_marked = true;
            }
        }
        holes
    }

    #[inline]
    pub fn count_holes_and_avail_lines(&self, line_mark_state: u8) -> (usize, usize) {
        let mut holes = 0;
        let mut lines = 0;
        let mut prev_line_is_marked = true;
        for line in self.lines() {
            if !line.is_marked(line_mark_state) {
                lines += 1;
                if prev_line_is_marked {
                    holes += 1;
                }
                prev_line_is_marked = false;
            } else {
                prev_line_is_marked = true;
            }
        }
        (holes, lines)
    }
}

unsafe impl Step for Block {
    #[inline(always)]
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        debug_assert!(!super::BLOCK_ONLY);
        if start < end { return None }
        Some((end.start() - start.start()) >> Self::LOG_BYTES)
    }
    #[inline(always)]
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Self::from(start.start() + (count << Self::LOG_BYTES)))
    }
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Self::from(start.start() - (count << Self::LOG_BYTES)))
    }
}


struct Node<T> {
    value: T,
    next: AtomicPtr<Node<T>>,
}

pub struct BlockList {
    head: AtomicPtr<Node<Block>>,
    len: AtomicUsize,
    sync: Mutex<()>,
}

impl BlockList {
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::default(),
            len: AtomicUsize::default(),
            sync: Mutex::new(()),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn push(&self, block: Block) {
        let node = Box::leak(box Node { value: block, next: AtomicPtr::default() });
        loop {
            let next = self.head.load(Ordering::SeqCst);
            node.next.store(next, Ordering::SeqCst);
            if self.head.compare_exchange(next, node, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                break
            }
        }
        self.len.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn pop(&self) -> Option<Block> {
        loop {
            let head = self.head.load(Ordering::SeqCst);
            if head.is_null() { return None }
            let next = unsafe { (*head).next.load(Ordering::SeqCst) };
            if self.head.compare_exchange(head, next, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                let block = unsafe { (*head).value };
                unsafe { Box::from_raw(head) };
                self.len.fetch_sub(1, Ordering::SeqCst);
                return Some(block);
            }
        }
    }

    #[inline]
    pub fn reset(&self) {
        let _guard = self.sync.lock().unwrap();
        self.len.store(0, Ordering::SeqCst);
        loop {
            let head = self.head.load(Ordering::SeqCst);
            if head.is_null() { return }
            self.head.store(unsafe { (*head).next.load(Ordering::SeqCst) }, Ordering::SeqCst);
            unsafe { Box::from_raw(head) };
        }
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item=Block> {
        BlockListIter { head: self.head.load(Ordering::SeqCst) }
    }
}

impl<'a> IntoIterator for &'a BlockList {
    type Item = Block;
    type IntoIter = impl Iterator<Item=Self::Item>;

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
                    self.head.store(node.next.load(Ordering::SeqCst), Ordering::SeqCst);
                    unsafe { Box::from_raw(node_ptr) };
                    return Some(block);
                } else {
                    self.head = &node.next;
                }
            }
        }
    }
}
