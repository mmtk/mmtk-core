use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use std::sync::Mutex;
use util::statistics::Timer;
use util::statistics::counter::{Counter, LongCounter};
use util::statistics::counter::MonotoneNanoTime;

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

fn new_counter<T: Counter + Send + 'static>(c: T) -> usize {
    let mut counter = COUNTER.lock().unwrap();
    counter.push(Box::new(c));
    return counter.len();
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
    total_time: usize
}

impl Stats {
    pub fn start_gc(&mut self) {
        let mut counter = COUNTER.lock().unwrap();
        self.gc_count += 1;
        if !get_gathering_stats() {
            return;
        }
        if get_phase() < MAX_PHASES - 1 {
            counter[self.total_time].phase_change(get_phase());
            increment_phase();
        } else {
            if !EXCEEDED_PHASE_LIMIT.load(Ordering::SeqCst) {
                println!("Warning: number of GC phases exceeds MAX_PHASES");
                EXCEEDED_PHASE_LIMIT.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn end_gc(&mut self) {
        let mut counter = COUNTER.lock().unwrap();
        if !get_gathering_stats() {
            return;
        }
        if get_phase() < MAX_PHASES - 1 {
            counter[self.total_time].phase_change(get_phase());
            increment_phase();
        } else {
            if !EXCEEDED_PHASE_LIMIT.load(Ordering::SeqCst) {
                println!("Warning: number of GC phases exceeds MAX_PHASES");
                EXCEEDED_PHASE_LIMIT.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn print_stats(&self) {
        let mut counter = COUNTER.lock().unwrap();
        println!("============================ MMTk Statistics Totals ============================");
        self.print_column_names();
        print!("{}\t", get_phase() / 2);
        if counter[self.total_time].merge_phases() {
            counter[self.total_time].print_total(None);
        } else {
            counter[self.total_time].print_total(Some(true));
            print!("\t");
            counter[self.total_time].print_total(Some(false));
            print!("\t");
        }
        println!();
        print!("Total time: ");
        counter[self.total_time].print_total(None);
        println!(" ms");
        println!("------------------------------ End MMTk Statistics -----------------------------")
    }

    pub fn print_column_names(&self) {
        println!("GC\ttime.mu\ttime.gc");
    }

    pub fn start_all(&mut self) {
        let mut counter = COUNTER.lock().unwrap();
        if get_gathering_stats() {
            println!("Error: calling Stats.startAll() while stats running");
            println!("       verbosity > 0 and the harness mechanism may be conflicting");
            debug_assert!(false);
        }
        set_gathering_stats(true);
        if counter[self.total_time].implicitly_start() {
            counter[self.total_time].start()
        }
    }

    pub fn stop_all(&mut self) {
        self.stop_all_counters();
        self.print_stats();
    }

    pub fn stop_all_counters(&mut self) {
        let mut counter = COUNTER.lock().unwrap();
        counter[self.total_time].stop();
        set_gathering_stats(false);
    }

    pub fn new() -> Self {
        let t: Timer = LongCounter::new("totalTime".to_string(), true, false);
        Stats {
            gc_count: 0,
            total_time: new_counter(t)
        }
    }
}
