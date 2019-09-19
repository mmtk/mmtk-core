mod region;
mod regionspace;
pub mod cardtable;
mod card;
mod remset;
mod marktable;

pub use self::region::*;
pub use self::regionspace::*;
pub use self::card::*;
pub use self::remset::*;
pub use self::marktable::*;
pub use self::cardtable::*;

const DEBUG: bool = false;

pub trait AccumulativePauseTimePredictor: Sized {
    fn record(&mut self, r: RegionRef);
    fn predict(&self) -> usize;
    fn predict_f32(&self) -> f32;
    /// Return true if the pause time is within the budget
    fn within_budget(&self) -> bool;
}

use std::sync::atomic::{AtomicUsize, Ordering};

pub struct PauseTimePredictionTimer {
    v_fixed: AtomicUsize,
    u: AtomicUsize,
    u_cards: AtomicUsize,
    s: AtomicUsize,
    s_cards: AtomicUsize,
    c: AtomicUsize,
    c_bytes: AtomicUsize,
    pause_start: ::std::time::SystemTime,
    num_workers: usize,
}

impl PauseTimePredictionTimer {
    pub fn new() -> Self {
        Self {
            v_fixed: AtomicUsize::new(0),
            u: AtomicUsize::new(0),
            u_cards: AtomicUsize::new(0),
            s: AtomicUsize::new(0),
            s_cards: AtomicUsize::new(0),
            c: AtomicUsize::new(0),
            c_bytes: AtomicUsize::new(0),
            pause_start: ::std::time::UNIX_EPOCH,
            num_workers: 0,
        }
    }
    fn reset(&self) {
        self.v_fixed.store(0, Ordering::Relaxed);
        self.u.store(0, Ordering::Relaxed); // Total Card Refine Time
        self.u_cards.store(0, Ordering::Relaxed);
        self.s.store(0, Ordering::Relaxed); // Totoal RS Card Scan Time
        self.s_cards.store(0, Ordering::Relaxed);
        self.c.store(0, Ordering::Relaxed); // Total Copy Time
        self.c_bytes.store(0, Ordering::Relaxed);
    }
    pub fn pause_start(&mut self, num_workers: usize) {
        self.reset();
        self.pause_start = ::std::time::SystemTime::now();
        self.num_workers = num_workers;
    }
    pub fn pause_end(&self, full_gc: bool) -> usize {
        let total_pause_time = self.pause_start.elapsed().unwrap().as_millis() as usize;
        let ud = self.u.load(Ordering::Relaxed) / self.num_workers;
        let vs = self.s.load(Ordering::Relaxed) / self.num_workers;
        let vc = self.c.load(Ordering::Relaxed) / self.num_workers;
        if !full_gc {
            let v_fixed = total_pause_time - ud - vs - vc;
            self.v_fixed.store(v_fixed, Ordering::SeqCst);
        }
        total_pause_time
    }
    pub fn report_dirty_card_scanning_time(&self, time: usize, cards: usize) {
        self.u.fetch_add(time, Ordering::Relaxed);
        self.u_cards.fetch_add(cards, Ordering::Relaxed);
    }
    pub fn report_remset_card_scanning_time(&self, time: usize, cards: usize) {
        self.s.fetch_add(time, Ordering::Relaxed);
        self.s_cards.fetch_add(cards, Ordering::Relaxed);
    }
    pub fn report_evacuation_time(&self, time: usize, bytes: usize) {
        self.c.fetch_add(time, Ordering::Relaxed);
        self.c_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
    pub fn v_fixed(&self) -> usize { self.v_fixed.load(Ordering::Relaxed) }
    pub fn u(&self) -> f32 { self.u.load(Ordering::Relaxed) as f32 / self.u_cards.load(Ordering::Relaxed) as f32 }
    // pub fn d(&self) -> f32 { self.u_cards.load(Ordering::Relaxed) as f32 }
    pub fn s(&self) -> f32 { self.s.load(Ordering::Relaxed) as f32 / self.s_cards.load(Ordering::Relaxed) as f32 }
    pub fn c(&self) -> f32 { self.c.load(Ordering::Relaxed) as f32 / self.c_bytes.load(Ordering::Relaxed) as f32 }
}
