use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use ::util::statistics::counter::Counter;
use std::sync::Mutex;

lazy_static! {
    pub static ref STATS: Stats = Stats::new();
    static ref COUNTER: Mutex<Vec<Box<Counter + Send>>> = Mutex::new(Vec::new());
}

// FIXME overflow detection
static PHASE: AtomicUsize = AtomicUsize::new(0);
static COUNTERS: AtomicUsize = AtomicUsize::new(0);
static GATHERING_STATS: AtomicBool = AtomicBool::new(false);

pub const MAX_PHASES: usize = 1 << 12;
pub const MAX_COUNTERS: usize = 100;

fn increment_phase() {
    PHASE.fetch_add(1, Ordering::SeqCst);
}

pub fn get_phase() -> usize {
    PHASE.load(Ordering::SeqCst)
}

pub fn get_gathering_stats() -> bool {
    GATHERING_STATS.load(Ordering::SeqCst)
}

pub fn set_gathering_stats(val: bool) {
    GATHERING_STATS.store(val, Ordering::SeqCst);
}

pub struct Stats {
    gc_count: usize
}

impl Stats {
    pub fn start_gc(&mut self) {
        self.gc_count += 1;
    }

    pub fn end_gc(&mut self) {}

    pub fn print_stats(&self) {
        println!("========================= Rust MMTk Statistics Totals =========================");
        println!("GC Count: {}", self.gc_count);
        println!("----------------------- End Rust MMTk Statistics Totals -----------------------")
    }

    pub fn new() -> Self {
        Stats {
            gc_count: 0
        }
    }
}
