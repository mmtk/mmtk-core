use std::collections::HashSet;
use std::mem::swap;
use std::sync::Mutex;

use crate::util::Address;

pub struct TreadMill {
    from_space: Mutex<HashSet<Address>>,
    to_space: Mutex<HashSet<Address>>,
    collect_nursery: Mutex<HashSet<Address>>,
    alloc_nursery: Mutex<HashSet<Address>>,
}

impl std::fmt::Debug for TreadMill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreadMill")
            .field("from", &self.from_space.lock().unwrap())
            .field("to", &self.to_space.lock().unwrap())
            .field("collect_nursery", &self.collect_nursery.lock().unwrap())
            .field("alloc_nursery", &self.alloc_nursery.lock().unwrap())
            .finish()
    }
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
        if nursery {
            // println!("+ an {}", cell);
            self.alloc_nursery.lock().unwrap().insert(cell);
        } else {
            // println!("+ ts {}", cell);
            self.to_space.lock().unwrap().insert(cell);
        }
    }

    pub fn collect_nursery(&self) -> Vec<Address> {
        let mut guard = self.collect_nursery.lock().unwrap();
        let vals = guard.iter().copied().collect();
        guard.clear();
        drop(guard);
        vals
    }

    pub fn collect(&self) -> Vec<Address> {
        let mut guard = self.from_space.lock().unwrap();
        let vals = guard.iter().copied().collect();
        guard.clear();
        drop(guard);
        vals
    }

    pub fn copy(&self, cell: Address, is_in_nursery: bool) {
        if is_in_nursery {
            let mut guard = self.collect_nursery.lock().unwrap();
            // debug_assert!(
            //     guard.contains(&cell),
            //     "copy source cell ({}) must be in collect_nursery",
            //     cell
            // );
            guard.remove(&cell);
            // println!("cn -> ts {}", cell);
        } else {
            let mut guard = self.from_space.lock().unwrap();
            // debug_assert!(
            //     guard.contains(&cell),
            //     "copy source cell ({}) must be in from_space",
            //     cell
            // );
            guard.remove(&cell);
            // println!("fs -> ts {}", cell);
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
        // println!("an <-> cn");
        if full_heap {
            swap(&mut self.from_space, &mut self.to_space);
            // println!("fs <-> ts");
        }
    }
}

impl Default for TreadMill {
    fn default() -> Self {
        Self::new()
    }
}
