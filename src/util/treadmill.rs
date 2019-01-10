use std::collections::hash_set::Iter;
use std::collections::HashSet;
use std::mem::swap;
use std::sync::Mutex;

use ::util::{Address, ObjectReference};

#[derive(Debug)]
pub struct TreadMill {
    from_space: Mutex<HashSet<Address>>,
    to_space: Mutex<HashSet<Address>>,
    collect_nursery: Mutex<HashSet<Address>>,
    alloc_nursery: Mutex<HashSet<Address>>,
}

impl TreadMill {
    pub fn new() -> Self {
        TreadMill {
            from_space: Mutex::new(HashSet::new()),
            to_space: Mutex::new(HashSet::new()),
            collect_nursery: Mutex::new(HashSet::new()),
            alloc_nursery: Mutex::new(HashSet::new()),
        }
    }

    pub fn add_to_treadmill(&self, cell: Address, nursery: bool) {
        println!("Adding {:?} to treadmill", cell);
        if nursery {
            self.alloc_nursery.lock().unwrap().insert(cell);
        } else {
            self.to_space.lock().unwrap().insert(cell);
        }
    }

    pub fn iter_nursery(&self) -> Vec<Address> {
        let guard = self.collect_nursery.lock().unwrap();
        let vals = guard.iter().map(|x|*x).collect();
        drop(guard);
        vals
    }

    pub fn iter(&self) -> Vec<Address> {
        let guard = self.from_space.lock().unwrap();
        let vals = guard.iter().map(|x|*x).collect();
        drop(guard);
        vals
    }

    pub fn copy(&mut self, cell: Address, is_in_nursery: bool) {
        if is_in_nursery {
            let mut guard = self.collect_nursery.lock().unwrap();
            debug_assert!(guard.contains(&cell));
            guard.remove(&cell);
        } else {
            let mut guard = self.from_space.lock().unwrap();
            debug_assert!(guard.contains(&cell));
            guard.remove(&cell);
        }
        self.to_space.lock().unwrap().insert(cell);
    }

    pub fn to_space_empty(&self) -> bool {
        self.to_space.lock().unwrap().is_empty()
    }

    pub fn from_space_empty(&self) -> bool {
        self.from_space.lock().unwrap().is_empty()
    }

    pub fn nursery_empty(&self) -> bool {
        self.collect_nursery.lock().unwrap().is_empty()
    }

    pub fn flip(&mut self, full_heap: bool) {
        swap(&mut self.alloc_nursery, &mut self.collect_nursery);
        if full_heap {
            swap(&mut self.from_space, &mut self.to_space);
        }
    }
}