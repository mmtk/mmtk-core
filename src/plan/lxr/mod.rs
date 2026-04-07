mod barrier;
pub(super) mod cm;
mod gc_work;
pub(super) mod global;
mod mature_evac;
pub(super) mod mutator;
pub mod rc;
mod remset;

use std::sync::atomic::AtomicUsize;

pub use self::global::LXR;
pub use self::global::LXR_CONSTRAINTS;
pub use self::remset::MatureEvecRemSet;

use atomic::Atomic;
use atomic::Ordering;

const CYCLE_TRIGGER_THRESHOLD: usize = crate::args::CYCLE_TRIGGER_THRESHOLD;

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
        // println!("set_alloc_size {}", size);
        gc_log!([2] " - alloc vol = {}", size);
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
