mod barrier;
mod block_allocation;
pub(crate) mod block_sweeping;
pub(super) mod cm;
mod gc_work;
pub(super) mod global;
pub(crate) mod los_work;
mod mature_evac;
pub(super) mod mutator;
pub mod rc;
pub(crate) mod rc_work;
mod remset;

use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

pub use self::global::LXR;
pub use self::global::LXR_CONSTRAINTS;
pub use self::remset::MatureEvecRemSet;

use atomic::Atomic;
use atomic::Ordering;
use spin::Lazy;
type RwLock<T> = spin::rwlock::RwLock<T>;

// --- LXR-specific global state ---

pub(crate) static NUM_CONCURRENT_TRACING_PACKETS: AtomicUsize = AtomicUsize::new(0);
pub(crate) static DISABLE_LASY_DEC_FOR_CURRENT_GC: AtomicBool = AtomicBool::new(false);
pub(crate) static NO_EVAC: AtomicBool = AtomicBool::new(false);

// --- LXR-specific global constants/flags ---

/// Enable Lazy Decrements
pub(crate) const LAZY_DECREMENTS: bool = !cfg!(feature = "lxr_no_lazy");

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

pub(crate) fn concurrent_marking_packets_drained() -> bool {
    NUM_CONCURRENT_TRACING_PACKETS.load(Ordering::SeqCst) == 0
}

pub(crate) fn disable_lasy_dec_for_current_gc() -> bool {
    DISABLE_LASY_DEC_FOR_CURRENT_GC.load(Ordering::SeqCst)
}
// --- Lazy sweeping job counters ---

pub(crate) struct LazySweepingJobsCounter {
    decs_counter: Option<Arc<AtomicUsize>>,
    counter: Arc<AtomicUsize>,
}
impl LazySweepingJobsCounter {
    pub fn new() -> Self {
        let lazy_sweeping_jobs = LAZY_SWEEPING_JOBS.read();
        let counter = lazy_sweeping_jobs.curr_counter.as_ref().unwrap();
        counter.fetch_add(1, Ordering::SeqCst);
        Self {
            decs_counter: None,
            counter: counter.clone(),
        }
    }

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

pub(crate) struct LazySweepingJobs {
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

pub(crate) static LAZY_SWEEPING_JOBS: Lazy<RwLock<LazySweepingJobs>> =
    Lazy::new(|| RwLock::new(LazySweepingJobs::new()));

pub static SURVIVAL_RATIO_PREDICTOR: SurvivalRatioPredictor = SurvivalRatioPredictor {
    prev_ratio: Atomic::new(0.01),
    alloc_vol: AtomicUsize::new(0),
    copy_promote_vol: AtomicUsize::new(0),
};

pub struct SurvivalRatioPredictor {
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

pub struct SurvivalRatioPredictorLocal {
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

pub static MATURE_LIVE_PREDICTOR: MatureLivePredictor = MatureLivePredictor {
    live_pages: Atomic::new(0f64),
};

pub struct MatureLivePredictor {
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
