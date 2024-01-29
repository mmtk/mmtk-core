use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// This stores some global states for an MMTK instance.
/// Some MMTK components like plans and allocators may keep an reference to the struct, and can access it.
// This used to be a part of the `BasePlan`. In that case, any component that accesses
// the states needs a reference to the plan. It makes it harder for us to reason about the access pattern
// for the plan, as many components hold references to the plan. Besides, the states
// actually are not related with a plan, they are just global states for MMTK. So we refactored
// those fields to this separate struct. For components that access the state, they just need
// a reference to the struct, and are no longer dependent on the plan.
// We may consider further break down the fields into smaller structs.
pub struct GlobalState {
    /// Whether MMTk is now ready for collection. This is set to true when initialize_collection() is called.
    pub(crate) initialized: AtomicBool,
    /// The current GC status.
    pub(crate) gc_status: Mutex<GcStatus>,
    /// Is the current GC an emergency collection? Emergency means we may run out of memory soon, and we should
    /// attempt to collect as much as we can.
    pub(crate) emergency_collection: AtomicBool,
    /// Is the current GC triggered by the user?
    pub(crate) user_triggered_collection: AtomicBool,
    /// Is the current GC triggered internally by MMTK? This is unused for now. We may have internally triggered GC
    /// for a concurrent plan.
    pub(crate) internal_triggered_collection: AtomicBool,
    /// Is the last GC internally triggered?
    pub(crate) last_internal_triggered_collection: AtomicBool,
    // Has an allocation succeeded since the emergency collection?
    pub(crate) allocation_success: AtomicBool,
    // Maximum number of failed attempts by a single thread
    pub(crate) max_collection_attempts: AtomicUsize,
    // Current collection attempt
    pub(crate) cur_collection_attempts: AtomicUsize,
    /// A counter for per-mutator stack scanning
    pub(crate) scanned_stacks: AtomicUsize,
    /// Have we scanned all the stacks?
    pub(crate) stacks_prepared: AtomicBool,
    /// A counter that keeps tracks of the number of bytes allocated since last stress test
    pub(crate) allocation_bytes: AtomicUsize,
    /// A counteer that keeps tracks of the number of bytes allocated by malloc
    #[cfg(feature = "malloc_counted_size")]
    pub(crate) malloc_bytes: AtomicUsize,
    /// This stores the size in bytes for all the live objects in last GC. This counter is only updated in the GC release phase.
    #[cfg(feature = "count_live_bytes_in_gc")]
    pub(crate) live_bytes_in_last_gc: AtomicUsize,
}

impl GlobalState {
    /// Is MMTk initialized?
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    /// Set the collection kind for the current GC. This is called before
    /// scheduling collection to determin what kind of collection it will be.
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
            gc_status: Mutex::new(GcStatus::NotInGC),
            stacks_prepared: AtomicBool::new(false),
            emergency_collection: AtomicBool::new(false),
            user_triggered_collection: AtomicBool::new(false),
            internal_triggered_collection: AtomicBool::new(false),
            last_internal_triggered_collection: AtomicBool::new(false),
            allocation_success: AtomicBool::new(false),
            max_collection_attempts: AtomicUsize::new(0),
            cur_collection_attempts: AtomicUsize::new(0),
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
