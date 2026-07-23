use atomic_refcell::AtomicRefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

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
    /// The current GC status.
    pub(crate) gc_status: GcStatusWord,
    /// When did the last GC start? Only accessed by the last parked worker.
    pub(crate) gc_start_time: AtomicRefCell<Option<Instant>>,
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
    /// Are we inside the benchmark harness?
    pub(crate) inside_harness: AtomicBool,
    /// A counteer that keeps tracks of the number of bytes allocated by malloc
    #[cfg(feature = "malloc_counted_size")]
    pub(crate) malloc_bytes: AtomicUsize,
    /// This stores the live bytes and the used bytes (by pages) for each space in last GC. This counter is only updated in the GC release phase.
    pub(crate) live_bytes_in_last_gc: AtomicRefCell<HashMap<&'static str, LiveBytesStats>>,
    /// The number of used pages at the end of the last GC. This can be used to estimate how many pages we have allocated since last GC.
    pub(crate) used_pages_after_last_gc: AtomicUsize,
}

impl GlobalState {
    /// Is MMTk initialized?
    pub fn is_initialized(&self) -> bool {
        self.gc_status.is_initialized()
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

    pub(crate) fn set_used_pages_after_last_gc(&self, pages: usize) {
        self.used_pages_after_last_gc
            .store(pages, Ordering::Relaxed);
    }

    pub(crate) fn get_used_pages_after_last_gc(&self) -> usize {
        self.used_pages_after_last_gc.load(Ordering::Relaxed)
    }
}

impl Default for GlobalState {
    fn default() -> Self {
        Self {
            gc_status: GcStatusWord::new(GcStatus::Uninitialized),
            gc_start_time: AtomicRefCell::new(None),
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
            inside_harness: AtomicBool::new(false),
            #[cfg(feature = "malloc_counted_size")]
            malloc_bytes: AtomicUsize::new(0),
            live_bytes_in_last_gc: AtomicRefCell::new(HashMap::new()),
            used_pages_after_last_gc: AtomicUsize::new(0),
        }
    }
}

/// The status of MMTk's GC subsystem. This doubles as the "is MMTk initialized" flag (via
/// [`GcStatus::Uninitialized`]) and as the state that tracks whether a GC is running and, if so,
/// what phase it is in. See [`GcStatusWord`] for how this is stored atomically and which
/// transitions between variants are legal.
#[derive(PartialEq, Copy, Clone, Debug)]
pub enum GcStatus {
    /// MMTk has not been initialized yet, i.e. `initialize_collection()` has not been called, so
    /// there are no GC worker threads available to run a collection. [`GcStatusWord::try_request_pause`]
    /// returns `Err(GcStatus::Uninitialized)` exactly when the status is `GcStatus::Uninitialized`.
    Uninitialized,
    /// MMTk is initialized, and no GC is running, pending, or requested.
    NotInGC,
    /// A concurrent GC's background work (e.g. concurrent marking) is running while mutators
    /// continue to run normally. See `ConcurrentPlan::concurrent_work_in_progress`.
    InConcurrentGC,
    /// A stop-the-world pause is active: mutators are stopped and GC workers are doing pause
    /// work (e.g. tracing).
    InPause,
    /// A GC pause has been requested (by [`GcStatusWord::try_request_pause`]) but mutators have
    /// not all stopped yet.
    PauseRequested,
    /// Collection is currently disabled (e.g. by a mutator calling `disable_collection()`).
    /// The usize payload is the non-zero nesting depth of disable calls:
    /// each call to `disable_collection()` increments the depth, and each call to `enable_collection()`
    /// decrements it. When the depth is about to reach zero, the status is transitioned to `NoInGC`.
    Disabled(usize),
}

/// A lock-free, atomic encoding of [`GcStatus`]. This packs the variant tag into the low bits
/// of a `usize` and, for `GcStatus::Disabled`, the nesting depth into the remaining high bits,
/// so the whole status fits in a single machine word and can be updated with compare-and-swap
/// instead of behind a `Mutex<GcStatus>`.
///
/// `GcStatus` is a state machine: only a handful of transitions between its variants are legal.
/// Every legal transition is exposed here as its own method, each performing its own
/// compare-and-swap retry loop and asserting that the transition is legal for the status it
/// finds. Do not add a generic "set the status to X" method: doing so would make it possible to
/// bypass the state machine's invariants.
pub(crate) struct GcStatusWord(AtomicUsize);

impl GcStatusWord {
    /// Number of bits used to encode the variant tag. 3 bits is enough to distinguish the 6
    /// variants, leaving the rest of the word for `Disabled`'s nesting depth.
    const TAG_BITS: u32 = 3;
    const TAG_MASK: usize = (1 << Self::TAG_BITS) - 1;

    fn encode(status: GcStatus) -> usize {
        match status {
            GcStatus::Uninitialized => 0,
            GcStatus::NotInGC => 1,
            GcStatus::InConcurrentGC => 2,
            GcStatus::InPause => 3,
            GcStatus::PauseRequested => 4,
            GcStatus::Disabled(depth) => {
                debug_assert!(
                    depth < (1 << (usize::BITS - Self::TAG_BITS)),
                    "GC-disable nesting depth overflows the bits reserved for it"
                );
                5 | (depth << Self::TAG_BITS)
            }
        }
    }

    fn decode(bits: usize) -> GcStatus {
        match bits & Self::TAG_MASK {
            0 => GcStatus::Uninitialized,
            1 => GcStatus::NotInGC,
            2 => GcStatus::InConcurrentGC,
            3 => GcStatus::InPause,
            4 => GcStatus::PauseRequested,
            5 => GcStatus::Disabled(bits >> Self::TAG_BITS),
            _ => unreachable!("invalid encoded GcStatus tag"),
        }
    }

    pub(crate) fn new(status: GcStatus) -> Self {
        GcStatusWord(AtomicUsize::new(Self::encode(status)))
    }

    /// Read the current status.
    pub(crate) fn load(&self) -> GcStatus {
        Self::decode(self.0.load(Ordering::SeqCst))
    }

    /// Retry `f` (a pure function of the current status) via [`AtomicUsize::fetch_update`] until
    /// it succeeds, and return the status it transitioned *from* (not the new status). Returning
    /// the old status (rather than the new one) lets a caller tell whether it "won" the race when
    /// multiple threads concurrently drive the same transition: only the thread whose CAS
    /// actually moved the status away from a given old value can be sure it is the one
    /// responsible for that transition, so it is the one that should perform any side effect that
    /// must happen exactly once (e.g. notifying the scheduler). If `transition` returned the new
    /// status instead, every racing thread would observe the same new status and none could tell
    /// which of them caused it. `f` may be invoked more than once under contention.
    fn transition<F: FnMut(GcStatus) -> GcStatus>(&self, mut f: F) -> GcStatus {
        let old_bits = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |bits| {
                Some(Self::encode(f(Self::decode(bits))))
            })
            .unwrap(); // `f` always returns a status to move to, so this never returns `Err`.
        Self::decode(old_bits)
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.load() != GcStatus::Uninitialized
    }

    pub(crate) fn is_disabled(&self) -> bool {
        matches!(self.load(), GcStatus::Disabled(_))
    }

    /// `Uninitialized` -> `NotInGC`.
    pub(crate) fn set_initialized(&self) {
        self.transition(|status| {
            assert!(
                status == GcStatus::Uninitialized,
                "Trying to set initialized GC status when it is not uninitialized"
            );
            GcStatus::NotInGC
        });
    }

    /// Any status other than `Uninitialized` -> `Uninitialized`.
    pub(crate) fn set_uninitialized(&self) {
        self.transition(|status| {
            assert!(
                status != GcStatus::Uninitialized,
                "Trying to set uninitialized GC status when it is already uninitialized"
            );
            GcStatus::Uninitialized
        });
    }

    /// `PauseRequested` -> `InPause`.
    pub(crate) fn set_in_pause(&self) {
        self.transition(|status| {
            assert!(
                status == GcStatus::PauseRequested,
                "Trying to set in-pause GC status in invalid status: {:?}",
                status
            );
            GcStatus::InPause
        });
    }

    /// `InPause` -> `InConcurrentGC`, e.g. once a GC pause has finished but concurrent work (such
    /// as concurrent marking) was scheduled to continue after mutators resume.
    pub(crate) fn set_in_concurrent_gc(&self) {
        self.transition(|status| {
            assert!(
                status == GcStatus::InPause,
                "Trying to set in-concurrent-gc GC status in invalid status: {:?}",
                status
            );
            GcStatus::InConcurrentGC
        });
    }

    /// `InPause` -> `NotInGC`, e.g. once a GC pause has finished and no concurrent work remains.
    pub(crate) fn set_not_in_gc(&self) {
        self.transition(|status| {
            assert!(
                status == GcStatus::InPause,
                "Trying to set not-in-gc GC status in invalid status: {:?}",
                status
            );
            GcStatus::NotInGC
        });
    }

    /// `NotInGC`/`Disabled(depth)` -> `Disabled(depth + 1)`. Returns `false` without changing the
    /// status if collection cannot be disabled from the current status (e.g. a GC is in progress
    /// or has been requested).
    pub(crate) fn set_disabled(&self) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |bits| {
                let next = match Self::decode(bits) {
                    GcStatus::Disabled(depth) => GcStatus::Disabled(depth + 1),
                    GcStatus::NotInGC => GcStatus::Disabled(1),
                    _ => return None,
                };
                Some(Self::encode(next))
            })
            .is_ok()
    }

    /// `Disabled(depth)` -> `Disabled(depth - 1)`, or `Disabled(1)` -> `NotInGC`. If collection is
    /// not currently disabled, this is a no-op (the status is left unchanged). Returns `true` if
    /// this call actually re-enabled collection (i.e. it was the outermost `Disabled(1)` ->
    /// `NotInGC` transition), `false` if it only decremented the nesting depth, or if collection
    /// was already enabled.
    pub(crate) fn set_enabled(&self) -> bool {
        let old = self.transition(|status| match status {
            GcStatus::Disabled(1) => GcStatus::NotInGC,
            GcStatus::Disabled(depth) => GcStatus::Disabled(depth - 1),
            other => other,
        });
        old == GcStatus::Disabled(1)
    }

    /// `NotInGC`/`InConcurrentGC` -> `PauseRequested`, unless collection is disabled, MMTk is not
    /// yet initialized, or a pause has already been requested, in which case `Err` is returned
    /// with the status that prevented the transition (`Disabled(_)`, `Uninitialized`, or
    /// `PauseRequested` respectively).
    pub(crate) fn try_request_pause(&self) -> Result<(), GcStatus> {
        // `fetch_update`'s closure returning `None` aborts the update and makes `fetch_update`
        // return `Err` with the status that caused the abort, so `Disabled`/`Uninitialized`/
        // `PauseRequested` (which must not transition here) are reported that way instead of via
        // a CAS.
        match self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |bits| {
                let status = Self::decode(bits);
                if matches!(
                    status,
                    GcStatus::Disabled(_) | GcStatus::Uninitialized | GcStatus::PauseRequested
                ) {
                    return None;
                }
                assert!(
                    matches!(status, GcStatus::NotInGC | GcStatus::InConcurrentGC),
                    "Trying to request a GC pause in invalid status: {:?}",
                    status
                );
                Some(Self::encode(GcStatus::PauseRequested))
            }) {
            Ok(_) => Ok(()),
            Err(bits) => {
                let error_state = Self::decode(bits);
                assert!(
                    matches!(
                        error_state,
                        GcStatus::Disabled(_) | GcStatus::Uninitialized | GcStatus::PauseRequested
                    ),
                    "fetch_update aborted the transition for an unexpected status: {:?}",
                    error_state
                );
                Err(error_state)
            }
        }
    }
}

#[cfg(test)]
mod gc_status_tests {
    use super::{GcStatus, GcStatusWord};

    #[test]
    fn encode_decode_roundtrip() {
        let statuses = [
            GcStatus::Uninitialized,
            GcStatus::NotInGC,
            GcStatus::InConcurrentGC,
            GcStatus::InPause,
            GcStatus::PauseRequested,
            GcStatus::Disabled(1),
            GcStatus::Disabled(42),
        ];
        for status in statuses {
            assert_eq!(GcStatusWord::decode(GcStatusWord::encode(status)), status);
        }
    }

    #[test]
    fn new_and_load_roundtrip() {
        let statuses = [
            GcStatus::Uninitialized,
            GcStatus::NotInGC,
            GcStatus::InConcurrentGC,
            GcStatus::InPause,
            GcStatus::PauseRequested,
            GcStatus::Disabled(1),
            GcStatus::Disabled(42),
        ];
        for status in statuses {
            assert_eq!(GcStatusWord::new(status).load(), status);
        }
    }

    #[test]
    fn set_initialized_from_uninitialized() {
        let word = GcStatusWord::new(GcStatus::Uninitialized);
        assert!(!word.is_initialized());
        word.set_initialized();
        assert_eq!(word.load(), GcStatus::NotInGC);
        assert!(word.is_initialized());
    }

    #[test]
    #[should_panic(expected = "not uninitialized")]
    fn set_initialized_panics_if_already_initialized() {
        GcStatusWord::new(GcStatus::NotInGC).set_initialized();
    }

    #[test]
    fn set_uninitialized_from_not_in_gc() {
        let word = GcStatusWord::new(GcStatus::NotInGC);
        word.set_uninitialized();
        assert_eq!(word.load(), GcStatus::Uninitialized);
    }

    #[test]
    #[should_panic(expected = "already uninitialized")]
    fn set_uninitialized_panics_if_already_uninitialized() {
        GcStatusWord::new(GcStatus::Uninitialized).set_uninitialized();
    }

    #[test]
    fn try_request_pause_from_not_in_gc() {
        let word = GcStatusWord::new(GcStatus::NotInGC);
        assert!(word.try_request_pause().is_ok());
        assert_eq!(word.load(), GcStatus::PauseRequested);
    }

    #[test]
    fn try_request_pause_from_in_concurrent_gc() {
        let word = GcStatusWord::new(GcStatus::InConcurrentGC);
        assert!(word.try_request_pause().is_ok());
        assert_eq!(word.load(), GcStatus::PauseRequested);
    }

    #[test]
    fn try_request_pause_when_already_requested() {
        let word = GcStatusWord::new(GcStatus::PauseRequested);
        assert_eq!(word.try_request_pause(), Err(GcStatus::PauseRequested));
        // Idempotent: the status is unchanged, not "double requested".
        assert_eq!(word.load(), GcStatus::PauseRequested);
    }

    /// A GC pause should never be requested while one is already underway: by the time the
    /// status reaches `InPause`, all mutators must already be stopped, so no mutator should be
    /// calling `try_request_pause` at all. Observing `InPause` here indicates a state-machine
    /// violation elsewhere, so it must panic rather than being silently treated as a no-op.
    #[test]
    #[should_panic(expected = "invalid status")]
    fn try_request_pause_panics_when_already_in_pause() {
        let _ = GcStatusWord::new(GcStatus::InPause).try_request_pause();
    }

    #[test]
    fn try_request_pause_when_disabled() {
        let word = GcStatusWord::new(GcStatus::Disabled(1));
        assert_eq!(word.try_request_pause(), Err(GcStatus::Disabled(1)));
        // Unchanged: disabling is not overridden by a pause request.
        assert_eq!(word.load(), GcStatus::Disabled(1));
    }

    /// Allocation can call `poll()` (and thus `try_request_pause`) before
    /// `initialize_collection()` has been called, e.g. if the heap fills up before the VM
    /// binding initializes MMTk's GC worker threads. This must not panic here: the caller (e.g.
    /// `Space::not_acquiring`) is responsible for producing a clear "GC is not allowed here"
    /// error once it knows allocation has genuinely failed.
    #[test]
    fn try_request_pause_when_uninitialized() {
        let word = GcStatusWord::new(GcStatus::Uninitialized);
        assert_eq!(word.try_request_pause(), Err(GcStatus::Uninitialized));
        assert_eq!(word.load(), GcStatus::Uninitialized);
    }

    #[test]
    fn set_in_pause_from_pause_requested() {
        let word = GcStatusWord::new(GcStatus::PauseRequested);
        word.set_in_pause();
        assert_eq!(word.load(), GcStatus::InPause);
    }

    #[test]
    #[should_panic(expected = "invalid status")]
    fn set_in_pause_panics_if_not_requested() {
        GcStatusWord::new(GcStatus::NotInGC).set_in_pause();
    }

    #[test]
    fn set_disabled_from_not_in_gc() {
        let word = GcStatusWord::new(GcStatus::NotInGC);
        assert!(word.set_disabled());
        assert_eq!(word.load(), GcStatus::Disabled(1));
    }

    #[test]
    fn set_disabled_nests() {
        let word = GcStatusWord::new(GcStatus::Disabled(1));
        assert!(word.set_disabled());
        assert_eq!(word.load(), GcStatus::Disabled(2));
        assert!(word.set_disabled());
        assert_eq!(word.load(), GcStatus::Disabled(3));
    }

    #[test]
    fn set_disabled_fails_without_changing_status() {
        for status in [
            GcStatus::Uninitialized,
            GcStatus::InConcurrentGC,
            GcStatus::PauseRequested,
            GcStatus::InPause,
        ] {
            let word = GcStatusWord::new(status);
            assert!(!word.set_disabled());
            assert_eq!(word.load(), status);
        }
    }

    #[test]
    fn set_enabled_decrements_nesting() {
        let word = GcStatusWord::new(GcStatus::Disabled(3));
        assert!(!word.set_enabled());
        assert_eq!(word.load(), GcStatus::Disabled(2));
    }

    #[test]
    fn set_enabled_to_not_in_gc_at_zero_depth() {
        let word = GcStatusWord::new(GcStatus::Disabled(1));
        assert!(word.set_enabled());
        assert_eq!(word.load(), GcStatus::NotInGC);
    }

    #[test]
    fn set_disabled_and_set_enabled_nest_round_trip() {
        let word = GcStatusWord::new(GcStatus::NotInGC);
        assert!(word.set_disabled());
        assert!(word.set_disabled());
        assert!(word.set_disabled());
        assert_eq!(word.load(), GcStatus::Disabled(3));

        // Only the call that brings the nesting depth back to 0 (i.e. all the way back to
        // `NotInGC`) should return `true`.
        assert!(!word.set_enabled());
        assert_eq!(word.load(), GcStatus::Disabled(2));
        assert!(!word.set_enabled());
        assert_eq!(word.load(), GcStatus::Disabled(1));
        assert!(word.set_enabled());
        assert_eq!(word.load(), GcStatus::NotInGC);
    }

    #[test]
    fn set_enabled_is_noop_if_not_disabled() {
        for status in [
            GcStatus::Uninitialized,
            GcStatus::NotInGC,
            GcStatus::InConcurrentGC,
            GcStatus::InPause,
            GcStatus::PauseRequested,
        ] {
            let word = GcStatusWord::new(status);
            assert!(!word.set_enabled());
            assert_eq!(word.load(), status);
        }
    }

    #[test]
    fn set_in_concurrent_gc_from_in_pause() {
        let word = GcStatusWord::new(GcStatus::InPause);
        word.set_in_concurrent_gc();
        assert_eq!(word.load(), GcStatus::InConcurrentGC);
    }

    #[test]
    #[should_panic(expected = "invalid status")]
    fn set_in_concurrent_gc_panics_if_not_in_pause() {
        GcStatusWord::new(GcStatus::NotInGC).set_in_concurrent_gc();
    }

    #[test]
    fn set_not_in_gc_from_in_pause() {
        let word = GcStatusWord::new(GcStatus::InPause);
        word.set_not_in_gc();
        assert_eq!(word.load(), GcStatus::NotInGC);
    }

    #[test]
    #[should_panic(expected = "invalid status")]
    fn set_not_in_gc_panics_if_not_in_pause() {
        GcStatusWord::new(GcStatus::InConcurrentGC).set_not_in_gc();
    }

    #[test]
    fn is_disabled_reflects_status() {
        assert!(GcStatusWord::new(GcStatus::Disabled(1)).is_disabled());
        assert!(!GcStatusWord::new(GcStatus::NotInGC).is_disabled());
    }
}

/// Statistics for the live bytes in the last GC. The statistics is per space.
#[derive(Copy, Clone, Debug)]
pub struct LiveBytesStats {
    /// Total accumulated bytes of live objects in the space.
    pub live_bytes: usize,
    /// Total pages used by the space.
    pub used_pages: usize,
    /// Total bytes used by the space, computed from `used_pages`.
    /// The ratio of live_bytes and used_bytes reflects the utilization of the memory in the space.
    pub used_bytes: usize,
}
