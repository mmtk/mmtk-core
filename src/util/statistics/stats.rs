use crate::mmtk::MMTK;
use crate::util::statistics::counter::{Counter, LongCounter};
use crate::util::statistics::Timer;
use crate::vm::VMBinding;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use crate::plan::barriers::{ENABLE_BARRIER_COUNTER, BARRIER_COUNTER};

pub const MAX_PHASES: usize = 1 << 12;
pub const MAX_COUNTERS: usize = 100;

// Shared with each counter
pub struct SharedStats {
    phase: AtomicUsize,
    gathering_stats: AtomicBool,
}

impl SharedStats {
    fn increment_phase(&self) {
        self.phase.fetch_add(1, Ordering::SeqCst);
    }

    pub fn get_phase(&self) -> usize {
        self.phase.load(Ordering::SeqCst)
    }

    pub fn get_gathering_stats(&self) -> bool {
        self.gathering_stats.load(Ordering::SeqCst)
    }

    fn set_gathering_stats(&self, val: bool) {
        self.gathering_stats.store(val, Ordering::SeqCst);
    }
}

pub struct Stats {
    gc_count: AtomicUsize,
    total_time: Arc<Mutex<Timer>>,

    pub shared: Arc<SharedStats>,
    counters: Mutex<Vec<Arc<Mutex<dyn Counter + Send>>>>,
    exceeded_phase_limit: AtomicBool,
}

impl Stats {
    pub fn new() -> Self {
        let shared = Arc::new(SharedStats {
            phase: AtomicUsize::new(0),
            gathering_stats: AtomicBool::new(false),
        });
        let t = Arc::new(Mutex::new(LongCounter::new(
            "time".to_string(),
            shared.clone(),
            true,
            false,
        )));
        Stats {
            gc_count: AtomicUsize::new(0),
            total_time: t.clone(),

            shared,
            counters: Mutex::new(vec![t]),
            exceeded_phase_limit: AtomicBool::new(false),
        }
    }

    pub fn new_timer(
        &self,
        name: &str,
        implicit_start: bool,
        merge_phases: bool,
    ) -> Arc<Mutex<Timer>> {
        let mut guard = self.counters.lock().unwrap();
        let counter = Arc::new(Mutex::new(Timer::new(
            name.to_string(),
            self.shared.clone(),
            implicit_start,
            merge_phases,
        )));
        guard.push(counter.clone());
        counter
    }

    pub fn start_gc(&self) {
        self.gc_count.fetch_add(1, Ordering::SeqCst);
        if !self.get_gathering_stats() {
            return;
        }
        if self.get_phase() < MAX_PHASES - 1 {
            self.total_time
                .lock()
                .unwrap()
                .phase_change(self.get_phase());
            self.shared.increment_phase();
        } else if !self.exceeded_phase_limit.load(Ordering::SeqCst) {
            println!("Warning: number of GC phases exceeds MAX_PHASES");
            self.exceeded_phase_limit.store(true, Ordering::SeqCst);
        }
    }

    pub fn end_gc(&self) {
        if !self.get_gathering_stats() {
            return;
        }
        if self.get_phase() < MAX_PHASES - 1 {
            self.total_time
                .lock()
                .unwrap()
                .phase_change(self.get_phase());
            self.shared.increment_phase();
        } else if !self.exceeded_phase_limit.load(Ordering::SeqCst) {
            println!("Warning: number of GC phases exceeds MAX_PHASES");
            self.exceeded_phase_limit.store(true, Ordering::SeqCst);
        }
    }

    pub fn print_stats<VM: VMBinding>(&self, mmtk: &'static MMTK<VM>) {
        println!(
            "============================ MMTk Statistics Totals ============================"
        );
        let scheduler_stat = mmtk.scheduler.statistics();
        self.print_column_names(&scheduler_stat);
        print!("{}\t", self.get_phase() / 2);
        let counter = self.counters.lock().unwrap();
        for iter in &(*counter) {
            let c = iter.lock().unwrap();
            if c.merge_phases() {
                c.print_total(None);
                print!("\t");
            } else {
                c.print_total(Some(true));
                print!("\t");
                c.print_total(Some(false));
                print!("\t");
            }
        }
        for value in scheduler_stat.values() {
            print!("{}\t", value);
        }
        if ENABLE_BARRIER_COUNTER {
            let barrier_counter_results = BARRIER_COUNTER.get_results();
            print!("{:.5}\t", barrier_counter_results.take_rate);
            print!("{:.0}\t", barrier_counter_results.total);
            print!("{:.0}\t", barrier_counter_results.slow);
        }
        println!();
        print!("Total time: ");
        self.total_time.lock().unwrap().print_total(None);
        println!(" ms");
        println!("------------------------------ End MMTk Statistics -----------------------------")
    }

    pub fn print_column_names(&self, scheduler_stat: &HashMap<String, String>) {
        print!("GC\t");
        let counter = self.counters.lock().unwrap();
        for iter in &(*counter) {
            let c = iter.lock().unwrap();
            if c.merge_phases() {
                print!("{}\t", c.name());
            } else {
                print!("{}.mu\t{}.gc\t", c.name(), c.name());
            }
        }
        for name in scheduler_stat.keys() {
            print!("{}\t", name);
        }
        if ENABLE_BARRIER_COUNTER {
            print!("barrier.takerate\t");
            print!("barrier.total\t");
            print!("barrier.slow\t");
        }
        println!();
    }

    pub fn start_all(&self) {
        let _counter = self.counters.lock().unwrap();
        if self.get_gathering_stats() {
            println!("Error: calling Stats.startAll() while stats running");
            println!("       verbosity > 0 and the harness mechanism may be conflicting");
            debug_assert!(false);
        }
        self.shared.set_gathering_stats(true);

        let mut total_time_timer = self.total_time.lock().unwrap();
        if total_time_timer.implicitly_start {
            total_time_timer.start()
        }
        if ENABLE_BARRIER_COUNTER {
            BARRIER_COUNTER.reset();
        }
    }

    pub fn stop_all<VM: VMBinding>(&self, mmtk: &'static MMTK<VM>) {
        self.stop_all_counters();
        self.print_stats(mmtk);
    }

    fn stop_all_counters(&self) {
        let _counter = self.counters.lock().unwrap();
        self.total_time.lock().unwrap().stop();
        self.shared.set_gathering_stats(false);
    }

    fn get_phase(&self) -> usize {
        self.shared.get_phase()
    }

    pub fn get_gathering_stats(&self) -> bool {
        self.shared.get_gathering_stats()
    }
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}
