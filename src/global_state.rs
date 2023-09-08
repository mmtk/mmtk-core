use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct GlobalState {
    /// Whether MMTk is now ready for collection. This is set to true when initialize_collection() is called.
    pub initialized: AtomicBool,
    /// Should we trigger a GC when the heap is full? It seems this should always be true. However, we allow
    /// bindings to temporarily disable GC, at which point, we do not trigger GC even if the heap is full.
    pub trigger_gc_when_heap_is_full: AtomicBool,
    pub gc_status: Mutex<GcStatus>,
    pub emergency_collection: AtomicBool,
    pub user_triggered_collection: AtomicBool,
    pub internal_triggered_collection: AtomicBool,
    pub last_internal_triggered_collection: AtomicBool,
    // Has an allocation succeeded since the emergency collection?
    pub allocation_success: AtomicBool,
    // Maximum number of failed attempts by a single thread
    pub max_collection_attempts: AtomicUsize,
    // Current collection attempt
    pub cur_collection_attempts: AtomicUsize,
    #[cfg(feature = "sanity")]
    pub inside_sanity: AtomicBool,
    /// A counter for per-mutator stack scanning
    pub scanned_stacks: AtomicUsize,
    /// Have we scanned all the stacks?
    pub stacks_prepared: AtomicBool,
    /// A counter that keeps tracks of the number of bytes allocated since last stress test
    pub allocation_bytes: AtomicUsize,
    /// A counteer that keeps tracks of the number of bytes allocated by malloc
    #[cfg(feature = "malloc_counted_size")]
    pub malloc_bytes: AtomicUsize,
    /// This stores the size in bytes for all the live objects in last GC. This counter is only updated in the GC release phase.
    #[cfg(feature = "count_live_bytes_in_gc")]
    pub live_bytes_in_last_gc: AtomicUsize,
}

impl GlobalState {
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    pub fn should_trigger_gc_when_heap_is_full(&self) -> bool {
        self.trigger_gc_when_heap_is_full.load(Ordering::SeqCst)
    }

    pub fn set_collection_kind(
        &self,
        last_collection_was_exhaustive: bool,
        heap_can_grow: bool,
    ) -> bool {
        self.cur_collection_attempts.store(
            if self.user_triggered_collection.load(Ordering::Relaxed) {
                1
            } else {
                self.determine_collection_attempts()
            },
            Ordering::Relaxed,
        );

        let emergency_collection = !self.is_internal_triggered_collection()
            && last_collection_was_exhaustive
            && self.cur_collection_attempts.load(Ordering::Relaxed) > 1
            && !heap_can_grow;
        self.emergency_collection
            .store(emergency_collection, Ordering::Relaxed);

        emergency_collection
    }

    fn determine_collection_attempts(&self) -> usize {
        if !self.allocation_success.load(Ordering::Relaxed) {
            self.max_collection_attempts.fetch_add(1, Ordering::Relaxed);
        } else {
            self.allocation_success.store(false, Ordering::Relaxed);
            self.max_collection_attempts.store(1, Ordering::Relaxed);
        }

        self.max_collection_attempts.load(Ordering::Relaxed)
    }

    fn is_internal_triggered_collection(&self) -> bool {
        let is_internal_triggered = self
            .last_internal_triggered_collection
            .load(Ordering::SeqCst);
        // Remove this assertion when we have concurrent GC.
        assert!(
            !is_internal_triggered,
            "We have no concurrent GC implemented. We should not have internally triggered GC"
        );
        is_internal_triggered
    }

    pub fn is_emergency_collection(&self) -> bool {
        self.emergency_collection.load(Ordering::Relaxed)
    }

    /// Return true if this collection was triggered by application code.
    pub fn is_user_triggered_collection(&self) -> bool {
        self.user_triggered_collection.load(Ordering::Relaxed)
    }

    /// Reset collection state information.
    pub fn reset_collection_trigger(&self) {
        self.last_internal_triggered_collection.store(
            self.internal_triggered_collection.load(Ordering::SeqCst),
            Ordering::Relaxed,
        );
        self.internal_triggered_collection
            .store(false, Ordering::SeqCst);
        self.user_triggered_collection
            .store(false, Ordering::Relaxed);
    }

    /// Are the stacks scanned?
    pub fn stacks_prepared(&self) -> bool {
        self.stacks_prepared.load(Ordering::SeqCst)
    }

    /// Prepare for stack scanning. This is usually used with `inform_stack_scanned()`.
    /// This should be called before doing stack scanning.
    pub fn prepare_for_stack_scanning(&self) {
        self.scanned_stacks.store(0, Ordering::SeqCst);
        self.stacks_prepared.store(false, Ordering::SeqCst);
    }

    /// Inform that 1 stack has been scanned. The argument `n_mutators` indicates the
    /// total stacks we should scan. This method returns true if the number of scanned
    /// stacks equals the total mutator count. Otherwise it returns false. This method
    /// is thread safe and we guarantee only one thread will return true.
    pub fn inform_stack_scanned(&self, n_mutators: usize) -> bool {
        let old = self.scanned_stacks.fetch_add(1, Ordering::SeqCst);
        debug_assert!(
            old < n_mutators,
            "The number of scanned stacks ({}) is more than the number of mutators ({})",
            old,
            n_mutators
        );
        let scanning_done = old + 1 == n_mutators;
        if scanning_done {
            self.stacks_prepared.store(true, Ordering::SeqCst);
        }
        scanning_done
    }

    /// Increase the allocation bytes and return the current allocation bytes after increasing
    pub fn increase_allocation_bytes_by(&self, size: usize) -> usize {
        let old_allocation_bytes = self.allocation_bytes.fetch_add(size, Ordering::SeqCst);
        trace!(
            "Stress GC: old_allocation_bytes = {}, size = {}, allocation_bytes = {}",
            old_allocation_bytes,
            size,
            self.allocation_bytes.load(Ordering::Relaxed),
        );
        old_allocation_bytes + size
    }

    #[cfg(feature = "malloc_counted_size")]
    pub fn get_malloc_bytes_in_pages(&self) -> usize {
        crate::util::conversions::bytes_to_pages_up(self.malloc_bytes.load(Ordering::Relaxed))
    }

    #[cfg(feature = "malloc_counted_size")]
    pub(crate) fn increase_malloc_bytes_by(&self, size: usize) {
        self.malloc_bytes.fetch_add(size, Ordering::SeqCst);
    }

    #[cfg(feature = "malloc_counted_size")]
    pub(crate) fn decrease_malloc_bytes_by(&self, size: usize) {
        self.malloc_bytes.fetch_sub(size, Ordering::SeqCst);
    }

    #[cfg(feature = "count_live_bytes_in_gc")]
    pub fn get_live_bytes_in_last_gc(&self) -> usize {
        self.live_bytes_in_last_gc.load(Ordering::SeqCst)
    }

    #[cfg(feature = "count_live_bytes_in_gc")]
    pub fn set_live_bytes_in_last_gc(&self, size: usize) {
        self.live_bytes_in_last_gc.store(size, Ordering::SeqCst);
    }
}

impl Default for GlobalState {
    fn default() -> Self {
        Self {
            initialized: AtomicBool::new(false),
            trigger_gc_when_heap_is_full: AtomicBool::new(true),
            gc_status: Mutex::new(GcStatus::NotInGC),
            stacks_prepared: AtomicBool::new(false),
            emergency_collection: AtomicBool::new(false),
            user_triggered_collection: AtomicBool::new(false),
            internal_triggered_collection: AtomicBool::new(false),
            last_internal_triggered_collection: AtomicBool::new(false),
            allocation_success: AtomicBool::new(false),
            max_collection_attempts: AtomicUsize::new(0),
            cur_collection_attempts: AtomicUsize::new(0),
            #[cfg(feature = "sanity")]
            inside_sanity: AtomicBool::new(false),
            scanned_stacks: AtomicUsize::new(0),
            allocation_bytes: AtomicUsize::new(0),
            #[cfg(feature = "malloc_counted_size")]
            malloc_bytes: AtomicUsize::new(0),
            #[cfg(feature = "count_live_bytes_in_gc")]
            live_bytes_in_last_gc: AtomicUsize::new(0),
        }
    }
}

#[derive(PartialEq)]
pub enum GcStatus {
    NotInGC,
    GcPrepare,
    GcProper,
}
