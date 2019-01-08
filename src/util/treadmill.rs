use std::collections::hash_set::Iter;
use std::collections::HashSet;
use std::mem::swap;

use ::util::{Address, ObjectReference};

#[derive(Debug)]
pub struct TreadMill {
    from_space: HashSet<Address>,
    to_space: HashSet<Address>,
    collect_nursery: HashSet<Address>,
    alloc_nursery: HashSet<Address>,
}

impl TreadMill {
    pub fn new() -> Self {
        TreadMill {
            from_space: HashSet::new(),
            to_space: HashSet::new(),
            collect_nursery: HashSet::new(),
            alloc_nursery: HashSet::new(),
        }
    }

    pub fn add_to_treadmill(&mut self, cell: Address, nursery: bool) {
        if nursery {
            self.alloc_nursery.insert(cell);
        } else {
            self.to_space.insert(cell);
        }
    }

    pub fn iter_nursery(&self) -> Iter<Address> {
        self.collect_nursery.iter()
    }

    pub fn iter(&self) -> Iter<Address> {
        self.from_space.iter()
    }

    pub fn copy(&mut self, cell: Address, is_in_nursery: bool) {
        if is_in_nursery {
            self.collect_nursery.remove(&cell);
        } else {
            self.from_space.remove(&cell);
        }
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