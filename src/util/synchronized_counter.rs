use std::sync::atomic::{AtomicUsize, Ordering};
use std::usize;

pub struct SynchronizedCounter {
    count: AtomicUsize,
}

impl SynchronizedCounter {
    pub const fn new(init: usize) -> Self {
        Self {
            count: AtomicUsize::new(init),
        }
    }

    pub fn reset(&self) -> usize {
        self.count.swap(0, Ordering::Relaxed)
    }

    pub fn increment(&self) -> usize {
        debug_assert!(self.count.load(Ordering::Acquire) != usize::MAX);
        self.count.fetch_add(1, Ordering::Relaxed)
    }

    pub fn peek(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}
