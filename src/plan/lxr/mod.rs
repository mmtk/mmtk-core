mod barrier;
mod block_allocation;
mod gc_work;
pub(super) mod global;
mod mature_evac;
pub(super) mod mutator;

use crate::util::ObjectReference;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

pub use self::global::LXR;

use atomic::Atomic;
use atomic::Ordering;
use spin::Lazy;
type RwLock<T> = spin::rwlock::RwLock<T>;

// --- LXR-specific global state ---

static NUM_CONCURRENT_TRACING_PACKETS: AtomicUsize = AtomicUsize::new(0);
static DISABLE_LASY_DEC_FOR_CURRENT_GC: AtomicBool = AtomicBool::new(false);
static NO_EVAC: AtomicBool = AtomicBool::new(false);

/// Why the barrier declined to record a slot, for the `lxr_slot_skipped` USDT tracepoint.
/// Keep in sync with `tools/tracing/README.md`.
#[repr(usize)]
pub(crate) enum SkipReason {
    /// The slot lies outside the heap, so it has no field unlog bit and nothing keeps its
    /// memory alive until the increment is processed.
    OutOfHeap = 0,
    /// The slot is derived, so re-reading it at the pause would not yield its referent.
    /// See [`crate::vm::slot::Slot::is_derived`].
    Derived = 1,
}

// --- LXR-specific global constants/flags ---

/// Counts barrier slots that lie outside the heap and so have no field unlog bit. Such
/// a slot is recorded and re-read when its increment is processed, which is only sound
/// if the memory holding it is still there at that point.
pub(crate) static OUT_OF_HEAP_SLOTS: AtomicUsize = AtomicUsize::new(0);

/// Whether to validate the target of every increment before using it, reporting the
/// slot's provenance if it is not an object. Set `MMTK_LXR_CHECK_INCS=1`.
pub(crate) fn check_incs() -> bool {
    static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHECK.get_or_init(|| std::env::var_os("MMTK_LXR_CHECK_INCS").is_some())
}

/// Whether an address really holds an object, for the diagnostics above.
///
/// MMTk answers this from the valid-object bit, which is only maintained when built with
/// the `vo_bit` feature. There is no VM-neutral way to ask otherwise, so without it this
/// says yes to everything and the reports below go quiet. A binding that wants
/// `MMTK_LXR_CHECK_INCS` to find anything has to enable `vo_bit` too.
pub(crate) fn object_is_plausible(_o: ObjectReference) -> bool {
    #[cfg(feature = "vo_bit")]
    {
        crate::memory_manager::is_mmtk_object(_o.to_raw_address()).is_some()
    }
    #[cfg(not(feature = "vo_bit"))]
    {
        true
    }
}

/// The set of live objects as computed by the binding's verifier just before sweeping,
/// used to catch a decrement that takes a still-reachable object to zero *at the moment it
/// happens*, which is the only way to learn where that decrement came from.
///
/// Populated by [`set_live_set`] under `MMTK_LXR_VERIFY=1`; empty otherwise, and every
/// check against it is skipped when empty.
static LIVE_SET: std::sync::RwLock<Option<std::collections::HashSet<usize>>> =
    std::sync::RwLock::new(None);

/// Record the verifier's live set for the current pause. See [`LIVE_SET`].
pub fn set_live_set(objects: std::collections::HashSet<usize>) {
    *LIVE_SET.write().unwrap() = Some(objects);
}

/// Reference-counting ledger for objects the verifier says are live: object -> [(gc#, what
/// happened, the object whose field it was, the slot)]. Only live objects are tracked, which is
/// what keeps this affordable; without it a zero-count report can only show the GC worker's own
/// stack, which never names the store responsible.
type RcEvent = (usize, &'static str, usize, usize);
static DEC_SITES: std::sync::RwLock<Option<std::collections::HashMap<usize, Vec<RcEvent>>>> =
    std::sync::RwLock::new(None);

/// Dump the recorded ledger for `o`, most recent last. Empty output means no increment and no
/// decrement was ever applied to it while it was known live.
pub fn dump_rc_events(o: ObjectReference) {
    let guard = DEC_SITES.read().unwrap();
    let Some(events) = guard
        .as_ref()
        .and_then(|m| m.get(&o.to_raw_address().as_usize()))
    else {
        eprintln!("[lxr-verify]       ledger: empty (never inc'd or dec'd while live)");
        return;
    };
    for (gc, kind, src, slot) in events {
        eprintln!("[lxr-verify]       ledger: gc#{gc} {kind} src={src:#x} slot={slot:#x}");
    }
}

/// Record one reference-counting event against `old`. `kind` says what happened and where it
/// came from ("dec barrier", "inc mature", ...). Only called under [`check_incs`], and only for
/// objects the verifier knows to be live, which is what keeps the map small enough to afford.
///
/// The point is the *ledger*: an object that ends a pause with a zero count either received a
/// decrement it should not have, or never received an increment it was owed, and only the
/// sequence of events against that one object distinguishes the two.
pub(crate) fn record_rc_event(
    old: ObjectReference,
    kind: &'static str,
    src: Option<ObjectReference>,
    slot: crate::util::Address,
) {
    if !check_incs() {
        return;
    }
    // Normally only known-live objects are tracked, to keep this affordable. But an object's
    // *first* increment is the interesting one when it ends up uncounted, and a brand-new object
    // is not in the previous pause's live set, so that gate hides exactly what is being looked
    // for. Slots outside the heap -- fields of VM-space (sysimage) objects -- are few enough
    // (~16k per collection) to record unconditionally, and that is the class under suspicion.
    let slot_in_vm_space = {
        let layout = crate::util::heap::layout::vm_layout::vm_layout();
        !slot.is_zero() && (slot < layout.heap_start || slot >= layout.heap_end)
    };
    if !slot_in_vm_space && !is_known_live(old) {
        return;
    }
    let mut guard = DEC_SITES.write().unwrap();
    let map = guard.get_or_insert_with(Default::default);
    // Never reset per collection: the sites are recorded during the epoch *before* the pause
    // whose `STWRCDecsAndSweep` applies them, so a per-collection clear would discard exactly
    // the entries the report needs. Cap the size instead.
    if map.len() >= 4_000_000 {
        return;
    }
    let entry = map.entry(old.to_raw_address().as_usize()).or_default();
    if entry.len() < 32 {
        entry.push((
            GC_COUNT.load(Ordering::SeqCst),
            kind,
            src.map(|s| s.to_raw_address().as_usize()).unwrap_or(0),
            slot.as_usize(),
        ));
    }
}

/// Diagnostic: the coalescing state of the field at `a`, as the barrier sees it. `Some(true)`
/// means the field is logged, i.e. the barrier has already snapshotted it this epoch and will
/// skip further writes to it; `Some(false)` means the next write will be recorded; `None` means
/// the address has no field unlog bit at all.
///
/// Used by the bring-up verifier to ask, of an edge that was never counted, whether the barrier
/// believed it had already handled it.
pub fn field_is_logged<VM: crate::vm::VMBinding>(a: crate::util::Address) -> Option<bool> {
    use crate::vm::ObjectModel;
    let spec = *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
        .as_spec()
        .extract_side_spec();
    if !crate::util::metadata::side_metadata::address_to_meta_address(&spec, a).is_mapped() {
        return None;
    }
    Some(unsafe { spec.load::<u8>(a) } == crate::util::metadata::log_bit::LOGGED_VALUE)
}

/// Diagnostic counterpart of [`field_is_logged`] for the per-object log bit, which is what a
/// binding whose inlined barrier cannot name the field (Julia's `mmtk_gc_wb_fast`) gates on.
/// `true` means unlogged, i.e. the fast path will take the slow path on the next write.
pub fn object_is_unlogged<VM: crate::vm::VMBinding>(o: ObjectReference) -> bool {
    use crate::vm::ObjectModel;
    VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.is_unlogged::<VM>(o, Ordering::SeqCst)
}

/// Report a decrement that took a live object to zero, with a backtrace naming its origin.
#[cold]
fn report_dec_of_live_object(o: ObjectReference, origin: &str) {
    static REPORTED: AtomicUsize = AtomicUsize::new(0);
    if REPORTED.fetch_add(1, Ordering::Relaxed) >= 3 {
        return;
    }
    eprintln!(
        "[lxr] gc#{} BUG: decremented a live object to zero: {:?} (decrements from: {})",
        GC_COUNT.load(Ordering::SeqCst),
        o,
        origin,
    );
    // The recording site is the interesting part; the worker's own stack is not.
    dump_rc_events(o);
}

/// Whether `o` is in the verifier's live set, i.e. reachable from the VM's roots.
fn is_known_live(o: ObjectReference) -> bool {
    let guard = LIVE_SET.read().unwrap();
    match guard.as_ref() {
        Some(set) => set.contains(&o.to_raw_address().as_usize()),
        None => false,
    }
}

/// The quantities these used to count are now USDT tracepoints, because one of them
/// (`lxr_inc_pushed`) sat on the barrier's fast path where a global atomic is a cache line
/// shared by every mutator thread on every reference store. See `tools/tracing/README.md`
/// for what each reports and how to aggregate them:
///
/// - `lxr_promote` -- promotions, i.e. zero-to-one count transitions.
/// - `lxr_inc_pushed` / `lxr_inc_processed` -- slots the barrier buffered against
///   increments actually applied. Every buffered slot must be processed in the pause that
///   follows: a logged field is not recorded again, so a dropped slot loses its increment
///   permanently while the matching decrement was already taken.
/// - `lxr_slot_skipped` -- slots the barrier declined, tagged with a [`SkipReason`].
///
/// Objects `SweepDeadCycles` reclaimed as unmarked cyclic garbage, against those it kept because
/// tracing had marked them. If the trace marked the heap, "kept" must dominate; the reverse means
/// the sweep is reclaiming the live heap because the marks it consults were never set. These stay
/// as counters because the verifier reads them programmatically, and they are GC-time rather than
/// mutator-time.
pub(crate) static SWEEP_ZEROED: AtomicUsize = AtomicUsize::new(0);
pub(crate) static SWEEP_KEPT_MARKED: AtomicUsize = AtomicUsize::new(0);

/// Cumulative `(zeroed, kept_marked)` from [`SWEEP_ZEROED`] / [`SWEEP_KEPT_MARKED`]. The plan's own
/// stats print runs inside `release`, which is *before* the sweep, so it always reports zeroes;
/// read this from a post-sweep hook instead.
pub fn sweep_dead_cycle_counts() -> (usize, usize) {
    (
        SWEEP_ZEROED.load(Ordering::Relaxed),
        SWEEP_KEPT_MARKED.load(Ordering::Relaxed),
    )
}

/// Collections started so far, so diagnostics can say which GC they belong to.
pub(crate) static GC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Read [`GC_COUNT`], so binding-side diagnostics can label their output with the same
/// collection number the plan's own `MMTK_LXR_STATS` lines use.
pub fn gc_count() -> usize {
    GC_COUNT.load(Ordering::SeqCst)
}

/// Set once `LXR::release` begins. VM-side sweeping runs from there and decides liveness
/// by reference count, so any increment processed after this point is read too late.
pub(crate) static RELEASE_STARTED: AtomicBool = AtomicBool::new(false);

/// Increment packets that ran after `LXR::release` began. Must be zero.
pub(crate) static INCS_AFTER_RELEASE: AtomicUsize = AtomicUsize::new(0);

/// Whether to force every pause to be `RefCount`, so nothing is ever traced. Isolates the
/// reference-counting path from tracing and mature sweeping. Leaks cycles; diagnostic only.
pub(crate) fn no_full_pauses() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("MMTK_LXR_NO_FULL").is_some())
}

/// Whether to retain every nursery block instead of reclaiming its free lines, so that
/// live-but-uncounted objects survive. Set `MMTK_LXR_RETAIN_NURSERY=1`. Leaks; bring-up
/// diagnostic only.
pub fn retain_nursery() -> bool {
    static RETAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RETAIN.get_or_init(|| std::env::var_os("MMTK_LXR_RETAIN_NURSERY").is_some())
}

/// Whether to retain every mature block instead of releasing the ones whose objects all have
/// a zero count. Set `MMTK_LXR_RETAIN_MATURE=1`. Independent of [`retain_nursery`] so that the
/// two sweeps can be disabled one at a time, which is what tells them apart as suspects.
/// Leaks; bring-up diagnostic only.
pub fn retain_mature() -> bool {
    static RETAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RETAIN.get_or_init(|| std::env::var_os("MMTK_LXR_RETAIN_MATURE").is_some())
}

/// Enable Lazy Decrements
const LAZY_DECREMENTS: bool = !cfg!(feature = "lxr_no_lazy");

/// Enable Nursery Evacuation
const NURSERY_EVACUATION: bool = !cfg!(feature = "lxr_no_nursery_evac");

/// Enable Mature Evacuation
pub(crate) const MATURE_EVACUATION: bool = !cfg!(feature = "lxr_no_mature_evac");

/// Stop triggering CM or RC pauses, and trigger Full GCs instead if the available heap after a RC pause is still small.
const RC_STOP_PERCENT: usize = 15;

/// Trigger an RC pause when the predicted max survival size is larger than this threshold.
const MAX_SURVIVAL_MB: usize = 128;

/// Trigger a concurrent marking cycle when the predicted mature size is larger than this threshold.
const TRACE_THRESHOLD: usize = 20;

/// Start a concurrent marking cycle when the available pages in the previous pause is smaller than this threshold.
const CYCLE_TRIGGER_THRESHOLD: usize = 1024;

fn concurrent_marking_packets_drained() -> bool {
    NUM_CONCURRENT_TRACING_PACKETS.load(Ordering::SeqCst) == 0
}

fn disable_lasy_dec_for_current_gc() -> bool {
    DISABLE_LASY_DEC_FOR_CURRENT_GC.load(Ordering::SeqCst)
}

// --- Lazy sweeping job counters ---

struct LazySweepingJobsCounter {
    decs_counter: Option<Arc<AtomicUsize>>,
    counter: Arc<AtomicUsize>,
}
impl LazySweepingJobsCounter {
    pub fn new_decs() -> Self {
        let lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.read();
        let decs_counter = lazy_sweeping_jobs.curr_decs_counter.as_ref().unwrap();
        decs_counter.fetch_add(1, Ordering::SeqCst);
        let counter = lazy_sweeping_jobs.curr_counter.as_ref().unwrap();
        counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: Some(decs_counter.clone()),
            counter: counter.clone(),
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn clone(&self) -> Self {
        self.counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: None,
            counter: self.counter.clone(),
        }
    }

    pub fn clone_with_decs(&self) -> Self {
        self.decs_counter
            .as_ref()
            .unwrap()
            .fetch_add(1, Ordering::SeqCst);
        self.counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: self.decs_counter.clone(),
            counter: self.counter.clone(),
        }
    }
}

impl Drop for LazySweepingJobsCounter {
    fn drop(&mut self) {
        let lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.read();
        if let Some(decs) = self.decs_counter.as_ref() {
            if decs.fetch_sub(1, Ordering::SeqCst) == 1 {
                let f = lazy_sweeping_jobs.end_of_decs.as_ref().unwrap();
                f(self.clone())
            }
        }
        if self.counter.fetch_sub(1, Ordering::SeqCst) == 1 {
            if let Some(f) = lazy_sweeping_jobs.end_of_lazy.as_ref() {
                f()
            }
        }
    }
}

struct LazySweepingJobs {
    prev_decs_counter: Option<Arc<AtomicUsize>>,
    curr_decs_counter: Option<Arc<AtomicUsize>>,
    prev_counter: Option<Arc<AtomicUsize>>,
    curr_counter: Option<Arc<AtomicUsize>>,
    pub end_of_decs: Option<Box<dyn Send + Sync + Fn(LazySweepingJobsCounter)>>,
    pub end_of_lazy: Option<Box<dyn Send + Sync + Fn()>>,
}

impl LazySweepingJobs {
    fn new() -> Self {
        Self {
            prev_decs_counter: None,
            curr_decs_counter: None,
            prev_counter: None,
            curr_counter: None,
            end_of_decs: None,
            end_of_lazy: None,
        }
    }

    pub fn all_finished() -> bool {
        LAZY_SWEEPING_JOBS
            .read()
            .prev_counter
            .as_ref()
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0)
            == 0
    }

    pub fn swap(&mut self) {
        self.prev_decs_counter = self.curr_decs_counter.take();
        self.curr_decs_counter = Some(Arc::new(AtomicUsize::new(0)));
        self.prev_counter = self.curr_counter.take();
        self.curr_counter = Some(Arc::new(AtomicUsize::new(0)));
    }
}

static LAZY_SWEEPING_JOBS: Lazy<RwLock<LazySweepingJobs>> =
    Lazy::new(|| RwLock::new(LazySweepingJobs::new()));

static SURVIVAL_RATIO_PREDICTOR: SurvivalRatioPredictor = SurvivalRatioPredictor {
    prev_ratio: Atomic::new(0.01),
    alloc_vol: AtomicUsize::new(0),
    copy_promote_vol: AtomicUsize::new(0),
};

struct SurvivalRatioPredictor {
    prev_ratio: Atomic<f64>,
    alloc_vol: AtomicUsize,
    copy_promote_vol: AtomicUsize,
}

impl SurvivalRatioPredictor {
    pub fn set_alloc_size(&self, size: usize) {
        assert_eq!(self.alloc_vol.load(Ordering::SeqCst), 0);
        self.alloc_vol.store(size, Ordering::SeqCst);
    }

    pub fn ratio(&self) -> f64 {
        self.prev_ratio.load(Ordering::Relaxed)
    }

    pub fn update_ratio(&self) -> f64 {
        if self.alloc_vol.load(Ordering::SeqCst) == 0 {
            self.copy_promote_vol.store(0, Ordering::SeqCst);
            return self.ratio();
        }
        let prev = self.prev_ratio.load(Ordering::SeqCst);
        let curr = self.copy_promote_vol.load(Ordering::SeqCst) as f64
            / self.alloc_vol.load(Ordering::SeqCst) as f64;
        let curr = f64::min(curr, 1.0);
        let ratio = (curr * 3f64 + prev) / 4f64;
        let ratio = f64::min(ratio, 1.0);
        self.prev_ratio.store(ratio, Ordering::SeqCst);
        self.alloc_vol.store(0, Ordering::SeqCst);
        self.copy_promote_vol.store(0, Ordering::SeqCst);
        ratio
    }
}

struct SurvivalRatioPredictorLocal {
    copy_promote_vol: AtomicUsize,
}

impl Default for SurvivalRatioPredictorLocal {
    fn default() -> Self {
        Self {
            copy_promote_vol: AtomicUsize::new(0),
        }
    }
}

impl SurvivalRatioPredictorLocal {
    pub fn record_copied_promotion(&self, size: usize) {
        self.copy_promote_vol.store(
            self.copy_promote_vol.load(Ordering::Relaxed) + size,
            Ordering::Relaxed,
        );
    }

    pub fn sync(&self) {
        SURVIVAL_RATIO_PREDICTOR.copy_promote_vol.fetch_add(
            self.copy_promote_vol.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
    }
}

static MATURE_LIVE_PREDICTOR: MatureLivePredictor = MatureLivePredictor {
    live_pages: Atomic::new(0f64),
};

struct MatureLivePredictor {
    live_pages: Atomic<f64>,
}

impl MatureLivePredictor {
    pub fn live_pages(&self) -> f64 {
        self.live_pages.load(Ordering::Relaxed)
    }

    pub fn update(&self, live_pages: usize) -> f64 {
        // println!("live_pages {}", live_pages);
        let prev = self.live_pages.load(Ordering::Relaxed);
        let curr = live_pages as f64;
        let weight = 3f64;
        let next = (weight * curr + prev) / (weight + 1f64);
        // println!("predict {}", next);
        // crate::add_mature_reclaim(live_pages, prev);
        self.live_pages.store(next, Ordering::Relaxed);
        next
    }
}
