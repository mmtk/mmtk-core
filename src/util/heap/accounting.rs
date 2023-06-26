use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

/// The struct is used for page usage.
/// Both page resource and side metadata uses this struct to do page accounting.
pub struct PageAccounting {
    /// The reserved pages. This should be incremented when we are about to allocate pages.
    /// Note this is different than quarantining address range. We do not count for quarantined
    /// memory.
    reserved: AtomicUsize,
    /// The committed pages. This should be incremented when we successfully allocate pages from the OS.
    committed: AtomicUsize,
}

impl PageAccounting {
    pub fn new() -> Self {
        Self {
            reserved: AtomicUsize::new(0),
            committed: AtomicUsize::new(0),
        }
    }

    /// Inform of both reserving and committing a certain number of pages.
    pub fn reserve_and_commit(&self, pages: usize) {
        self.reserved.fetch_add(pages, Ordering::Relaxed);
        self.committed.fetch_add(pages, Ordering::Relaxed);
    }

    /// Inform of reserving a certain number of pages. Usually this is called before attempting
    /// to allocate memory.
    pub fn reserve(&self, pages: usize) {
        self.reserved.fetch_add(pages, Ordering::Relaxed);
    }

    /// Inform of clearing some reserved pages. This is used when we have reserved some pages but
    /// the allocation cannot be satisfied. We can call this to clear the number of reserved pages,
    /// so later we can reserve and attempt again.
    pub fn clear_reserved(&self, pages: usize) {
        let _prev = self.reserved.fetch_sub(pages, Ordering::Relaxed);
        debug_assert!(_prev >= pages);
    }

    /// Inform of successfully committing a certain number of pages. This is used after we have reserved
    /// pages and successfully allocated those memory.
    pub fn commit(&self, pages: usize) {
        self.committed.fetch_add(pages, Ordering::Relaxed);
    }

    /// Inform of releasing a certain number of pages. The number of pages will be deducted from
    /// both reserved and committed pages.
    pub fn release(&self, pages: usize) {
        let _prev_reserved = self.reserved.fetch_sub(pages, Ordering::Relaxed);
        debug_assert!(_prev_reserved >= pages);

        let _prev_committed = self.committed.fetch_sub(pages, Ordering::Relaxed);
        debug_assert!(_prev_committed >= pages);
    }

    /// Set both reserved and committed pages to zero. This is only used when we completely clear a space.
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
