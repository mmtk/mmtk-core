mod barrier;
mod block_allocation;
mod gc_work;
pub(super) mod global;
mod mature_evac;
pub(super) mod mutator;

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

// --- LXR-specific global constants/flags ---

/// Collections started so far. Also identifies the cycle a wave of lazy sweeping jobs belongs
/// to; see `LazySweepingJobs::swap`.
pub(crate) static GC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// How many pending reference-count increments force a collection, bounding the `RCProcessIncs`
/// work a single pause has to do. `None` (spelled `0`) leaves the pause bounded only by heap
/// occupancy, which does not bound mutation at all.
///
/// Override with `MMTK_LXR_INC_BUFFER_LIMIT`, in entries. Each entry is one slot the barrier
/// recorded, so the limit is roughly "words of reference stores between pauses".
pub fn inc_buffer_limit() -> Option<usize> {
    static N: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        let v = match std::env::var("MMTK_LXR_INC_BUFFER_LIMIT") {
            Ok(v) => v.trim().parse().expect(
                "MMTK_LXR_INC_BUFFER_LIMIT must be a whole number of entries, or 0 for none",
            ),
            Err(_) => 0usize,
        };
        (v != 0).then_some(v)
    })
}

/// Smallest generation of recursively-discovered increments that `ProcessIncs` will split with
/// another worker instead of processing entirely itself. `usize::MAX` disables splitting, restoring
/// the chain. Override with `MMTK_LXR_SPLIT_MIN`.
pub fn active_packet_split() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| match std::env::var("MMTK_LXR_SPLIT_MIN") {
        Ok(v) => v
            .trim()
            .parse()
            .expect("MMTK_LXR_SPLIT_MIN must be a whole number of slots"),
        Err(_) => 64,
    })
}

/// How many nursery blocks a non-`Full` pause may sweep before handing the rest to the
/// concurrent phase. Override with `MMTK_LXR_STW_SWEEP_BLOCKS`; `usize::MAX` restores sweeping
/// the whole nursery inside the pause.
///
/// Sweeping a nursery block means `Block::rc_sweep_nursery`, which reads the block's reference
/// count table to decide whether the block is dead, reusable, or promoted in place. That is
/// O(nursery bytes) of work, it ran single-threaded in the `Release` packet, and it measured
/// 7-8ms of a 27ms `RefCount` pause on `tree_mutable` -- second only to `RCProcessIncs`.
///
/// None of it has to happen while the mutator is stopped. A block being swept is not yet on any
/// free list, so no mutator can allocate into it either way, and the concurrent phase that runs
/// it (`RCLazySweepNurseryBlocks`) already reports what it frees through
/// `num_clean_blocks_released_lazy`, which is what the next pause's sizing decision reads. The
/// only cost of deferring is that the pages come back a mutator window later.
///
/// A `Full` pause still sweeps everything itself: it is already a whole-heap stop, and it must
/// leave the heap in a state where nothing is owed to a concurrent phase.
pub fn max_stw_sweep_nursery_blocks() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| match std::env::var("MMTK_LXR_STW_SWEEP_BLOCKS") {
        Ok(v) => v
            .trim()
            .parse()
            .expect("MMTK_LXR_STW_SWEEP_BLOCKS must be a whole number of blocks"),
        Err(_) => 0,
    })
}

/// Enable Lazy Decrements
const LAZY_DECREMENTS: bool = !cfg!(feature = "lxr_no_lazy");

/// Enable Nursery Evacuation
const NURSERY_EVACUATION: bool = !cfg!(feature = "lxr_no_nursery_evac");

/// Enable Mature Evacuation
pub(crate) const MATURE_EVACUATION: bool = !cfg!(feature = "lxr_no_mature_evac");

/// Stop triggering CM or RC pauses, and trigger Full GCs instead if the available heap after a RC pause is still small.
const RC_STOP_PERCENT: usize = 15;

/// Trigger an RC pause when the predicted max survival size is larger than this threshold, in MB.
///
/// This is the bound that actually limits pause time. A pause's cost is dominated by
/// `RCProcessIncs` promoting the young objects that survived -- the bucket opens with a dozen
/// packets and fans out to thousands as the promotion trace walks their fields -- so the pause is
/// proportional to *surviving* young data, not to how much was allocated or how many references
/// were written. `inc_buffer_size` counts only the slots the barrier recorded and so does not bound
/// it; this does.
///
/// Override with `MMTK_LXR_MAX_SURVIVAL_MB`.
pub fn max_survival_mb() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| match std::env::var("MMTK_LXR_MAX_SURVIVAL_MB") {
        Ok(v) => v
            .trim()
            .parse()
            .expect("MMTK_LXR_MAX_SURVIVAL_MB must be a whole number of megabytes"),
        Err(_) => 128,
    })
}

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
                f(lazy_sweeping_jobs.epoch_of(&self.counter))
            }
        }
    }
}

/// The GC cycle a wave of lazy sweeping jobs belongs to.
///
/// A wave is everything that became owed between two consecutive [`LazySweepingJobs::swap`] calls,
/// and `swap` runs once per pause, so a wave corresponds one-to-one with a pause: the wave moved to
/// `prev` by the swap at the end of pause N is exactly the work pause N deferred. Draining it is
/// what makes pause N's reclamation visible, and that is the only moment the collector can size the
/// next heap target or judge its free headroom.
///
/// [`WAVE_STILL_OPEN`] marks the wave that is still accumulating (`curr`). It can hit zero
/// transiently -- every job so far has finished but more may still be added -- and such a moment
/// says nothing about any cycle being complete.
type WaveEpoch = usize;

const WAVE_STILL_OPEN: WaveEpoch = usize::MAX;

struct LazySweepingJobs {
    prev_decs_counter: Option<Arc<AtomicUsize>>,
    curr_decs_counter: Option<Arc<AtomicUsize>>,
    prev_counter: Option<Arc<AtomicUsize>>,
    curr_counter: Option<Arc<AtomicUsize>>,
    /// The cycle whose deferred work `prev_counter` covers. See [`WaveEpoch`].
    prev_epoch: WaveEpoch,
    pub end_of_decs: Option<Box<dyn Send + Sync + Fn(LazySweepingJobsCounter)>>,
    pub end_of_lazy: Option<Box<dyn Send + Sync + Fn(WaveEpoch)>>,
}

impl LazySweepingJobs {
    fn new() -> Self {
        Self {
            prev_decs_counter: None,
            curr_decs_counter: None,
            prev_counter: None,
            curr_counter: None,
            prev_epoch: WAVE_STILL_OPEN,
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

    /// Which wave a counter belongs to. Identified by pointer rather than carried in
    /// [`LazySweepingJobsCounter`] because a counter's wave is decided by the swap that closes it,
    /// which happens after the clones handed to individual work packets were made.
    fn epoch_of(&self, counter: &Arc<AtomicUsize>) -> WaveEpoch {
        match self.prev_counter.as_ref() {
            Some(prev) if Arc::ptr_eq(prev, counter) => self.prev_epoch,
            _ => WAVE_STILL_OPEN,
        }
    }

    /// Close the current wave, attributing it to the cycle `epoch` that is ending, and open a new
    /// one. Returns the number of jobs the closed wave still owes; zero means the cycle deferred
    /// nothing (or it has already all run), so nothing will report its completion later.
    pub fn swap(&mut self, epoch: WaveEpoch) -> usize {
        self.prev_decs_counter = self.curr_decs_counter.take();
        self.curr_decs_counter = Some(Arc::new(AtomicUsize::new(0)));
        self.prev_counter = self.curr_counter.take();
        self.curr_counter = Some(Arc::new(AtomicUsize::new(0)));
        self.prev_epoch = epoch;
        self.prev_counter
            .as_ref()
            .map(|c| c.load(Ordering::SeqCst))
            .unwrap_or(0)
    }
}

static LAZY_SWEEPING_JOBS: Lazy<RwLock<LazySweepingJobs>> =
    Lazy::new(|| RwLock::new(LazySweepingJobs::new()));

static SURVIVAL_RATIO_PREDICTOR: SurvivalRatioPredictor = SurvivalRatioPredictor {
    prev_ratio: Atomic::new(0.01),
    prev_promote_ratio: Atomic::new(0.01),
    alloc_vol: AtomicUsize::new(0),
    copy_promote_vol: AtomicUsize::new(0),
    promote_vol: AtomicUsize::new(0),
};

struct SurvivalRatioPredictor {
    prev_ratio: Atomic<f64>,
    /// As `prev_ratio`, but counting every promotion rather than only the copied ones. See
    /// [`SurvivalRatioPredictor::promotion_ratio`].
    prev_promote_ratio: Atomic<f64>,
    alloc_vol: AtomicUsize,
    copy_promote_vol: AtomicUsize,
    promote_vol: AtomicUsize,
}

impl SurvivalRatioPredictor {
    pub fn set_alloc_size(&self, size: usize) {
        assert_eq!(self.alloc_vol.load(Ordering::SeqCst), 0);
        self.alloc_vol.store(size, Ordering::SeqCst);
    }

    /// Fraction of young allocation that survived *by being copied*. This is what sizing a
    /// to-space wants, and it is legitimately zero when evacuation is off.
    pub fn ratio(&self) -> f64 {
        self.prev_ratio.load(Ordering::Relaxed)
    }

    /// Fraction of young allocation that survived at all, copied or promoted in place.
    ///
    /// This is the one to use to predict how much work the next pause will do, because a pause pays
    /// for every promotion: `ProcessIncs` scans each promoted object's fields and generates further
    /// increments from them, whether or not the object moved. [`Self::ratio`] cannot serve that
    /// purpose -- it counts only copied promotions, so with evacuation disabled (which is permanent
    /// for the Julia binding) it is pinned at zero. Anything predicting survival from it therefore
    /// predicted zero, which is how `MAX_SURVIVAL_MB` came to be unreachable: measured on
    /// `tree_mutable`, ~887k objects and ~27MB were promoted per pause while the prediction stayed
    /// at 0MB against a 128MB limit, so the bound that exists to cap pause time never once fired.
    pub fn promotion_ratio(&self) -> f64 {
        self.prev_promote_ratio.load(Ordering::Relaxed)
    }

    pub fn update_ratio(&self) -> f64 {
        if self.alloc_vol.load(Ordering::SeqCst) == 0 {
            self.copy_promote_vol.store(0, Ordering::SeqCst);
            self.promote_vol.store(0, Ordering::SeqCst);
            return self.ratio();
        }
        let alloc = self.alloc_vol.load(Ordering::SeqCst) as f64;
        let smooth = |prev: f64, curr: f64| {
            let curr = f64::min(curr, 1.0);
            f64::min((curr * 3f64 + prev) / 4f64, 1.0)
        };
        let ratio = smooth(
            self.prev_ratio.load(Ordering::SeqCst),
            self.copy_promote_vol.load(Ordering::SeqCst) as f64 / alloc,
        );
        let promote_ratio = smooth(
            self.prev_promote_ratio.load(Ordering::SeqCst),
            self.promote_vol.load(Ordering::SeqCst) as f64 / alloc,
        );
        self.prev_ratio.store(ratio, Ordering::SeqCst);
        self.prev_promote_ratio
            .store(promote_ratio, Ordering::SeqCst);
        self.alloc_vol.store(0, Ordering::SeqCst);
        self.copy_promote_vol.store(0, Ordering::SeqCst);
        self.promote_vol.store(0, Ordering::SeqCst);
        ratio
    }
}

struct SurvivalRatioPredictorLocal {
    copy_promote_vol: AtomicUsize,
    promote_vol: AtomicUsize,
}

impl Default for SurvivalRatioPredictorLocal {
    fn default() -> Self {
        Self {
            copy_promote_vol: AtomicUsize::new(0),
            promote_vol: AtomicUsize::new(0),
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

    /// Record a promotion of any kind. Per-worker and non-atomic like the above, published by
    /// [`Self::sync`], so it stays off the shared cache line that a global counter would put on the
    /// path of every promoted object.
    pub fn record_promotion(&self, size: usize) {
        self.promote_vol.store(
            self.promote_vol.load(Ordering::Relaxed) + size,
            Ordering::Relaxed,
        );
    }

    pub fn sync(&self) {
        SURVIVAL_RATIO_PREDICTOR.copy_promote_vol.fetch_add(
            self.copy_promote_vol.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        SURVIVAL_RATIO_PREDICTOR
            .promote_vol
            .fetch_add(self.promote_vol.load(Ordering::Relaxed), Ordering::Relaxed);
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
