use ::util::queue::LocalQueue;
use std::collections::HashMap;
use std::sync::{Condvar, Mutex};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::fmt::Debug;

type Block<T> = Vec<T>;

pub struct SharedQueue<T> {
    blocks: Mutex<Vec<Block<T>>>,
    count: AtomicUsize,
    bitmap: Mutex<HashMap<usize, bool>>,
}

impl<T> SharedQueue<T> where T: Debug {
    pub fn new() -> Self {
        SharedQueue {
            blocks: Mutex::new(Vec::new()),
            count: AtomicUsize::new(0),
            bitmap: Mutex::new(HashMap::new()),
        }
    }

    pub fn spin(&self, id: usize) -> Option<Block<T>> {
        // We are locally done
        let mut bitmap = self.bitmap.lock().unwrap();
        bitmap.insert(id, true);
        drop(bitmap);
        loop {
            let mut blocks = self.blocks.lock().unwrap();
            // Is the global queue empty?
            if !blocks.is_empty() {
                // No, we grab a new block
                let result = blocks.pop().unwrap();
                let mut bitmap = self.bitmap.lock().unwrap();
                // We are not done
                bitmap.insert(id, false);
                return Some(result);
            } else {
                // Yes
                let bitmap = self.bitmap.lock().unwrap();
                // Has anyone else finished?
                for (_, v) in bitmap.iter() {
                    if !v {
                        // No, we busy-wait
                        continue;
                    }
                }
                // Yes
                break;
            }
        }
        // Everyone is quiet
        return None;
    }

    pub fn push(&self, b: Block<T>) {
        let mut blocks = self.blocks.lock().unwrap();
        blocks.push(b);
    }

    pub fn spawn_local(&self) -> LocalQueue<T> {
        let mut bitmap = self.bitmap.lock().unwrap();
        let id = self.count.fetch_add(1, Ordering::SeqCst);
        bitmap.insert(id, false);
        LocalQueue::new(id, self)
    }

    pub fn is_empty(&self) -> bool {
        let blocks = self.blocks.lock().unwrap();
        blocks.is_empty()
    }

    pub fn clear(&self) {
        let mut blocks = self.blocks.lock().unwrap();
        blocks.clear();
    }
}