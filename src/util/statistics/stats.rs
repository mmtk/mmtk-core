use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::sync::Mutex;
use util::statistics::Timer;
use util::statistics::counter::{Counter, LongCounter};


lazy_static! {
    pub static ref STATS: Mutex<Box<Stats>> = Mutex::new(box Stats::new());
    static ref COUNTER: Mutex<Vec<Box<Counter + Send>>> = Mutex::new(Vec::new());
}

// FIXME overflow detection
static PHASE: AtomicUsize = AtomicUsize::new(0);
static COUNTERS: AtomicUsize = AtomicUsize::new(0);
static GATHERING_STATS: AtomicBool = AtomicBool::new(false);
static EXCEEDED_PHASE_LIMIT: AtomicBool = AtomicBool::new(false);

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
    gc_count: usize,
    total_time: Timer
}

impl Stats {
    pub fn start_gc(&mut self) {
        self.gc_count += 1;
        if !get_gathering_stats() {
            return;
        }
        if get_phase() < MAX_PHASES - 1 {
            self.total_time.phase_change(get_phase());
            increment_phase();
        } else {
            if !EXCEEDED_PHASE_LIMIT.load(Ordering::SeqCst) {
                println!("Warning: number of GC phases exceeds MAX_PHASES");
                EXCEEDED_PHASE_LIMIT.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn end_gc(&mut self) {
        if !get_gathering_stats() {
            return;
        }
        if get_phase() < MAX_PHASES - 1 {
            self.total_time.phase_change(get_phase());
            increment_phase();
        } else {
            if !EXCEEDED_PHASE_LIMIT.load(Ordering::SeqCst) {
                println!("Warning: number of GC phases exceeds MAX_PHASES");
                EXCEEDED_PHASE_LIMIT.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn print_stats(&self) {
        println!("========================= Rust MMTk Statistics Totals =========================");
        self.print_column_names();
        print!("{}\t", get_phase() / 2);
        if self.total_time.merge_phases() {
            self.total_time.print_total(None);
        } else {
            self.total_time.print_total(Some(true));
            print!("\t");
            self.total_time.print_total(Some(false));
            print!("\t");
        }
        println!();
        print!("Total time: ");
        self.total_time.print_total(None);
        println!(" ms");
        println!("----------------------- End Rust MMTk Statistics Totals -----------------------")
    }

    pub fn print_column_names(&self) {
        println!("GC\ttime.mu\ttime.gc");
    }

    pub fn start_all(&mut self) {
        if get_gathering_stats() {
            println!("Error: calling Stats.startAll() while stats running");
            println!("       verbosity > 0 and the harness mechanism may be conflicting");
            debug_assert!(false);
        }
        set_gathering_stats(true);
        if self.total_time.start {
            self.total_time.start()
        }
    }

    pub fn stop_all(&mut self) {
        self.stop_all_counters();
        self.print_stats();
    }

    pub fn stop_all_counters(&mut self) {
        self.total_time.stop();
        set_gathering_stats(false);
    }

    pub fn new() -> Self {
        Stats {
            gc_count: 0,
            total_time: LongCounter::new("totalTime", true, false)
        }
    }
}
