use crate::util::{Address, ObjectReference};
use crate::util::side_metadata::{self, *};
use crate::util::constants::*;
use std::{ops::Range, sync::{Mutex, MutexGuard, atomic::{AtomicPtr, Ordering}}};
use super::line::Line;
use crate::vm::*;



#[repr(u8)]
pub enum BlockMarkState {
    Unmarked = 0,
    Marked = 1,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
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

    #[inline]
    pub fn attempt_mark(&self) -> bool {
        side_metadata::compare_exchange_atomic(Self::MARK_TABLE, self.start(), BlockMarkState::Unmarked as usize, BlockMarkState::Marked as usize)
    }

    #[inline]
    pub fn mark(&self) {
        unsafe { side_metadata::store(Self::MARK_TABLE, self.start(), BlockMarkState::Marked as usize); }
    }

    #[inline]
    pub fn is_marked(&self) -> bool {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.start()) == BlockMarkState::Marked as usize }
    }

    #[inline]
    pub fn clear_mark(&self) {
        unsafe { side_metadata::store(Self::MARK_TABLE, self.start(), BlockMarkState::Unmarked as usize); }
    }

    pub const fn lines(&self) -> Range<Line> {
        debug_assert!(!super::BLOCK_ONLY);
        Range { start: Line::from(self.start()), end: Line::from(self.end()) }
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
