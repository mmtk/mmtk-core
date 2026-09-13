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

/// The current collection cycle number.
///
/// This is the epoch that identifies a wave of deferred lazy-sweeping jobs: a wave is stamped
/// with the value this held when the pause closed it, and only the wave whose epoch still
/// matches means "that cycle's reclamation is done". See [`LazySweepingJobs::swap`] and
/// `LXR::on_lazy_sweeping_finished`.
pub(crate) static GC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Enable Lazy Decrements
const LAZY_DECREMENTS: bool = !cfg!(feature = "lxr_no_lazy");

/// Enable Nursery Evacuation
const NURSERY_EVACUATION: bool = !cfg!(feature = "lxr_no_nursery_evac");

/// Enable Mature Evacuation
pub(crate) const MATURE_EVACUATION: bool = !cfg!(feature = "lxr_no_mature_evac");

/// Stop triggering CM or RC pauses, and trigger Full GCs instead if the available heap after a RC pause is still small.
const RC_STOP_PERCENT: usize = 15;

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
    alloc_vol: AtomicUsize::new(0),
    copy_promote_vol: AtomicUsize::new(0),
    prev_copy_promote_ratio: Atomic::new(0.01),
    promote_vol: AtomicUsize::new(0),
    prev_promote_ratio: Atomic::new(0.01),
};

/// Predicts how much of the young allocation in the coming cycle will survive.
struct SurvivalRatioPredictor {
    /// Young allocation over the current cycle: the denominator of both ratios.
    alloc_vol: AtomicUsize,
    /// Volume promoted by copying during the current cycle.
    copy_promote_vol: AtomicUsize,
    /// Smoothed `copy_promote_vol / alloc_vol` over previous cycles.
    prev_copy_promote_ratio: Atomic<f64>,
    /// Volume promoted by any means during the current cycle.
    promote_vol: AtomicUsize,
    /// Smoothed `promote_vol / alloc_vol` over previous cycles.
    prev_promote_ratio: Atomic<f64>,
}

impl SurvivalRatioPredictor {
    pub fn set_alloc_size(&self, size: usize) {
        assert_eq!(self.alloc_vol.load(Ordering::SeqCst), 0);
        self.alloc_vol.store(size, Ordering::SeqCst);
    }

    /// Fraction of young allocation that survived *by being copied*.
    pub fn copy_promote_ratio(&self) -> f64 {
        self.prev_copy_promote_ratio.load(Ordering::Relaxed)
    }

    /// Fraction of young allocation that survived at all, copied or promoted in place.
    pub fn promote_ratio(&self) -> f64 {
        self.prev_promote_ratio.load(Ordering::Relaxed)
    }

    pub fn update_ratios(&self) {
        let alloc_vol = self.alloc_vol.swap(0, Ordering::SeqCst);
        let copy_promote_vol = self.copy_promote_vol.swap(0, Ordering::SeqCst);
        let promote_vol = self.promote_vol.swap(0, Ordering::SeqCst);
        if alloc_vol == 0 {
            return;
        }
        let smooth = |prev_ratio: &Atomic<f64>, vol: usize| {
            let curr = f64::min(vol as f64 / alloc_vol as f64, 1.0);
            let prev = prev_ratio.load(Ordering::SeqCst);
            prev_ratio.store(f64::min((curr * 3f64 + prev) / 4f64, 1.0), Ordering::SeqCst);
        };
        smooth(&self.prev_copy_promote_ratio, copy_promote_vol);
        smooth(&self.prev_promote_ratio, promote_vol);
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
    pub fn record_promotion(&self, size: usize, copied: bool) {
        self.promote_vol.fetch_add(size, Ordering::Relaxed);
        if copied {
            self.copy_promote_vol.fetch_add(size, Ordering::Relaxed);
        }
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
