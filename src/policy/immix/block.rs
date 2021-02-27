use crate::util::{Address, ObjectReference};
use crate::util::side_metadata::{self, *};
use crate::util::constants::*;
use std::sync::atomic::{AtomicPtr, Ordering};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Block(Address);

impl Block {
    pub const LOG_PAGES_IN_BLOCK: usize = 3;
    pub const PAGES_IN_BLOCK: usize = 1 << Self::LOG_PAGES_IN_BLOCK;
    pub const LOG_BYTES_IN_BLOCK: usize = Self::LOG_PAGES_IN_BLOCK + LOG_BYTES_IN_PAGE as usize;
    pub const BYTES_IN_BLOCK: usize = 1 << Self::LOG_BYTES_IN_BLOCK;

    pub const MARK_TABLE: SideMetadataSpec = SideMetadataSpec {
        scope: SideMetadataScope::PolicySpecific,
        offset: 0,
        log_num_of_bits: 0,
        log_min_obj_size: Self::LOG_BYTES_IN_BLOCK,
    };

    pub const fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES_IN_BLOCK));
        Self(address)
    }

    pub const fn containing(object: ObjectReference) -> Self {
        Self(object.to_address().align_down(Self::BYTES_IN_BLOCK))
    }

    pub const fn start(&self) -> Address {
        self.0
    }

    pub const fn end(&self) -> Address {
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES_IN_BLOCK) }
    }

    pub fn mark(&self) -> bool {
        side_metadata::compare_exchange_atomic(Self::MARK_TABLE, self.start(), 0, 1)
    }

    pub fn is_marked(&self) -> bool {
        unsafe { side_metadata::load(Self::MARK_TABLE, self.start()) == 1 }
    }

    pub fn clear_mark(&self) {
        side_metadata::store_atomic(Self::MARK_TABLE, self.start(), 0);
    }
}


struct Node<T> {
    value: T,
    next: AtomicPtr<Node<T>>,
}

pub struct BlockList {
    head: AtomicPtr<Node<Block>>,
}

impl BlockList {
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::default(),
        }
    }

    pub fn add(&self, block: Block) {
        let mut node = Box::leak(box Node { value: block, next: AtomicPtr::default() });
        loop {
            let next = self.head.load(Ordering::SeqCst);
            node.next.store(next, Ordering::SeqCst);
            if self.head.compare_exchange(next, node, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                return
            }
        }
    }

    pub fn release(&self) {

    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item=Block> {
        BlockListIter { head: self.head.load(Ordering::SeqCst) }
    }

    #[inline]
    pub fn drain_filter<F: FnMut(&mut Block) -> bool>(&self, filter: F) -> DrainFilter<'_, F> {
        DrainFilter {
            head: &self.head,
            predicate: filter,
        }
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

pub struct DrainFilter<'a, F: 'a + FnMut(&mut Block) -> bool> {
    head: &'a AtomicPtr<Node<Block>>,
    predicate: F,
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
