use std::collections::VecDeque;
use std::mem::swap;

use ::util::Address;

#[derive(Debug)]
pub struct TreadMill {
    from_space: VecDeque<Address>,
    to_space: VecDeque<Address>,
    collect_nursery: VecDeque<Address>,
    alloc_nursery: VecDeque<Address>,
}

impl TreadMill {
    pub fn new() -> Self {
        TreadMill {
            from_space: VecDeque::new(),
            to_space: VecDeque::new(),
            collect_nursery: VecDeque::new(),
            alloc_nursery: VecDeque::new(),
        }
    }

    pub fn add_to_treadmill(&mut self, node: Address, nursery: bool) {
        if nursery {
            self.alloc_nursery.push_front(node);
        } else {
            self.to_space.push_front(node);
        }
    }

    pub fn pop_nursery(&mut self) -> Address {
        self.collect_nursery.pop_front().unwrap_or(unsafe { Address::zero() })
    }

    pub fn pop(&mut self) -> Address {
        self.from_space.pop_front().unwrap_or(unsafe { Address::zero() })
    }

    pub fn copy(&mut self, node: Address) {
        unimplemented!()
    }

    pub fn to_space_empty(&self) -> bool {
        self.to_space.is_empty()
    }

    pub fn from_space_empty(&self) -> bool {
        self.from_space.is_empty()
    }

    pub fn nursery_empty(&self) -> bool {
        self.collect_nursery.is_empty()
    }

    pub fn flip(&mut self, full_heap: bool) {
        swap(&mut self.alloc_nursery, &mut self.collect_nursery);
        if full_heap {
            swap(&mut self.from_space, &mut self.to_space);
        }
    }
}