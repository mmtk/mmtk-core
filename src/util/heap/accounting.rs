use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

/// The struct is used for page usage.
/// Both page resource and side metadata uses this struct to do page accounting.
pub struct PageAccounting {
    /// The reserved pages. This should be incremented when we are about to allocate pages.
    /// Note this is different than quarantining address range. We do not count for quarantined
    /// memory.
    reserved: AtomicUsize,
    /// The committed pages. This should be incremented when we succesfully allocate pages from the OS.
    committed: AtomicUsize,
}

impl PageAccounting {
    pub fn new() -> Self {
        Self {
            reserved: AtomicUsize::new(0),
            committed: AtomicUsize::new(0),
        }
    }

    pub fn reserve_and_commit(&self, pages: usize) {
        self.reserved.fetch_add(pages, Ordering::Relaxed);
        self.committed.fetch_add(pages, Ordering::Relaxed);
    }

    pub fn reserve(&self, pages: usize) {
        self.reserved.fetch_add(pages, Ordering::Relaxed);
    }

    pub fn clear_reserved(&self, pages: usize) {
        self.reserved.fetch_sub(pages, Ordering::Relaxed);
    }

    pub fn commit(&self, pages: usize) {
        self.committed.fetch_add(pages, Ordering::Relaxed);
    }

    pub fn release(&self, pages: usize) {
        self.reserved.fetch_sub(pages, Ordering::Relaxed);
        self.committed.fetch_sub(pages, Ordering::Relaxed);
    }

    pub fn reset(&self) {
        self.reserved.store(0, Ordering::Relaxed);
        self.committed.store(0, Ordering::Relaxed);
    }

    pub fn get_reserved_pages(&self) -> usize {
        self.reserved.load(Ordering::Relaxed)
    }

    pub fn get_committed_pages(&self) -> usize {
        self.committed.load(Ordering::Relaxed)
    }
}

impl Default for PageAccounting {
    fn default() -> Self {
        Self::new()
    }
}
