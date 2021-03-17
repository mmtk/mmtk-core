use crate::util::{Address, ObjectReference};
use crate::util::side_metadata::{self, *};
use crate::util::constants::*;
use std::{iter::Step, ops::Range, sync::{Mutex, MutexGuard, atomic::{AtomicPtr, Ordering}}};
use super::line::Line;
use super::chunk::Chunk;
use crate::vm::*;



#[repr(u8)]
#[derive(Debug, PartialEq)]
pub enum BlockState {
    Unallocated = 0,
    Unmarked = 1,
    Marked = 2,
    Reused = 3,
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

    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: if super::BLOCK_ONLY { 0 } else { Line::MARK_TABLE.accumulated_size() },
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

    // #[inline]
    // pub fn mark_as_defrag(&self) {
    //     debug_assert!(super::DEFRAG);
    //     self.set_mark_byte(BlockMarkState::Defrag as _);
    // }

    pub const fn line_mark_table(&self) -> Range<Address> {
        debug_assert!(!super::BLOCK_ONLY);
        let start = side_metadata::address_to_meta_address(Line::MARK_TABLE, self.start());
        let end = start + Block::LINES;
        start..end
    }

    // #[inline]
    // pub fn mark(&self, line_counter_increment: Option<usize>) {
    //     let state = if super::LINE_COUNTER {
    //         self.mark_byte() + line_counter_increment.unwrap() as u8
    //     } else {
    //         BlockMarkState::Marked as u8
    //     };
    //     self.set_mark_byte(state)
    // }

    // #[inline]
    // pub fn count_marked_lines(&self, line_mark_state: u8) -> usize {
    //     if super::LINE_COUNTER {
    //         self.mark_byte() as usize
    //     } else {
    //         self.lines().filter(|line| line.is_marked(line_mark_state)).count()
    //     }
    // }

    #[inline]
    pub fn get_state(&self) -> BlockState {
        unsafe {
            std::mem::transmute(self.mark_byte())
        }
    }

    #[inline]
    pub fn set_state(&self, state: BlockState) {
        self.set_mark_byte(state as _)
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
    sync: Mutex<()>,
}

impl BlockList {
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::default(),
            sync: Mutex::new(()),
        }
    }

    #[inline]
    pub fn push(&self, block: Block) {
        let node = Box::leak(box Node { value: block, next: AtomicPtr::default() });
        loop {
            let next = self.head.load(Ordering::SeqCst);
            node.next.store(next, Ordering::SeqCst);
            if self.head.compare_exchange(next, node, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                return
            }
        }
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
                return Some(block);
            }
        }
    }

    #[inline]
    pub fn reset(&self) {
        let _guard = self.sync.lock().unwrap();
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

    #[inline]
    pub fn drain_filter<'a, F: 'a + FnMut(&mut Block) -> bool>(&'a self, filter: F) -> impl 'a + Iterator<Item=Block> {
        DrainFilter {
            head: &self.head,
            predicate: filter,
            _guard: self.sync.lock().unwrap(),
        }
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
