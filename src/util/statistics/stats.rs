use crate::mmtk::MMTK;
use crate::util::options::Options;
use crate::util::statistics::counter::*;
use crate::util::statistics::Timer;
use crate::vm::VMBinding;

#[cfg(feature = "perf_counter")]
use pfm::Perfmon;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

pub const MAX_PHASES: usize = 1 << 14;
pub const MAX_COUNTERS: usize = 100;

/// GC stats shared among counters
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

/// GC statistics
///
/// The struct holds basic GC statistics, like the GC count,
/// and an array of counters.
pub struct Stats {
    gc_count: AtomicUsize,
    total_time: Arc<Mutex<Timer>>,
    // crate `pfm` uses libpfm4 under the hood for parsing perf event names
    // Initialization of libpfm4 is required before we can use `PerfEvent` types
    #[cfg(feature = "perf_counter")]
    perfmon: Perfmon,

    pub shared: Arc<SharedStats>,
    counters: Mutex<Vec<Arc<Mutex<dyn Counter + Send>>>>,
    exceeded_phase_limit: AtomicBool,
}

impl Stats {
    #[allow(unused)]
    pub fn new(options: &Options) -> Self {
        // Create a perfmon instance and initialize it
        // we use perfmon to parse perf event names
        #[cfg(feature = "perf_counter")]
        let perfmon = {
            let mut perfmon: Perfmon = Default::default();
            perfmon.initialize().expect("Perfmon failed to initialize");
            perfmon
        };
        let shared = Arc::new(SharedStats {
            phase: AtomicUsize::new(0),
            gathering_stats: AtomicBool::new(false),
        });
        let mut counters: Vec<Arc<Mutex<dyn Counter + Send>>> = vec![];
        // We always have a time counter enabled
        let t = Arc::new(Mutex::new(LongCounter::new(
            "time".to_string(),
            shared.clone(),
            true,
            false,
            MonotoneNanoTime {},
        )));
        counters.push(t.clone());
        // Read from the MMTK option for a list of perf events we want to
        // measure, and create corresponding counters
        #[cfg(feature = "perf_counter")]
        for e in &options.phase_perf_events.events {
            counters.push(Arc::new(Mutex::new(LongCounter::new(
                e.0.clone(),
                shared.clone(),
                true,
                false,
                PerfEventDiffable::new(&e.0),
            ))));
        }
        Stats {
            gc_count: AtomicUsize::new(0),
            total_time: t,
            #[cfg(feature = "perf_counter")]
            perfmon,

            shared,
            counters: Mutex::new(counters),
            exceeded_phase_limit: AtomicBool::new(false),
        }
    }

    pub fn new_event_counter(
        &self,
        name: &str,
        implicit_start: bool,
        merge_phases: bool,
    ) -> Arc<Mutex<EventCounter>> {
        let mut guard = self.counters.lock().unwrap();
        let counter = Arc::new(Mutex::new(EventCounter::new(
            name.to_string(),
            self.shared.clone(),
            implicit_start,
            merge_phases,
        )));
        guard.push(counter.clone());
        counter
    }

    pub fn new_size_counter(
        &self,
        name: &str,
        implicit_start: bool,
        merge_phases: bool,
    ) -> Mutex<SizeCounter> {
        let u = self.new_event_counter(name, implicit_start, merge_phases);
        let v = self.new_event_counter(&format!("{}.volume", name), implicit_start, merge_phases);
        Mutex::new(SizeCounter::new(u, v))
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
            MonotoneNanoTime {},
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
            let counters = self.counters.lock().unwrap();
            for counter in &(*counters) {
                counter.lock().unwrap().phase_change(self.get_phase());
            }
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
            let counters = self.counters.lock().unwrap();
            for counter in &(*counters) {
                counter.lock().unwrap().phase_change(self.get_phase());
            }
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
            } else {
                c.print_total(Some(true));
                print!("\t");
                c.print_total(Some(false));
            }
            print!("\t");
        }
        for value in scheduler_stat.values() {
            print!("{}\t", value);
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
                print!("{}.other\t{}.stw\t", c.name(), c.name());
            }
        }
        for name in scheduler_stat.keys() {
            print!("{}\t", name);
        }
        println!();
    }

    pub fn start_all(&self) {
        let counters = self.counters.lock().unwrap();
        if self.get_gathering_stats() {
            println!("Error: calling Stats.startAll() while stats running");
            println!("       verbosity > 0 and the harness mechanism may be conflicting");
            debug_assert!(false);
        }
        self.shared.set_gathering_stats(true);

        for c in &(*counters) {
            c.lock().unwrap().start();
        }
    }

    pub fn stop_all<VM: VMBinding>(&self, mmtk: &'static MMTK<VM>) {
        self.stop_all_counters();
        self.print_stats(mmtk);
    }

    fn stop_all_counters(&self) {
        let counters = self.counters.lock().unwrap();
        for c in &(*counters) {
            c.lock().unwrap().stop();
        }
        self.shared.set_gathering_stats(false);
    }

    fn get_phase(&self) -> usize {
        self.shared.get_phase()
    }

    pub fn get_gathering_stats(&self) -> bool {
        self.shared.get_gathering_stats()
    }
}
