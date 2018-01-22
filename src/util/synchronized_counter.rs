use std::usize;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SynchronizedCounter {
    count: AtomicUsize,
}

impl SynchronizedCounter {
    pub fn reset(&self) -> usize {
        self.count.swap(0, Ordering::Relaxed)
    }

    pub fn increment(&self) {
        debug_assert!(self.count.load(Ordering::Acquire) != usize::MAX);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn peek(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}