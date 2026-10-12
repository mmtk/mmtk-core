//! Connects [`FeedbackPacer`] to the plan: what the pacer is told, and when.
//!
//! The pacer decides when a cycle starts.  The heap size from the GC trigger (for example
//! `FixedHeapSize`, which is `-Xmx` on OpenJDK) stays the hard limit, and is also the pacer's
//! memory limit unless `soft_mem_limit` is set.

use super::cpu_limiter::CpuLimiter;
use super::pacer::FeedbackPacer;
use crate::util::options::Options;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

pub(super) struct Pacing {
    pub(super) pacer: FeedbackPacer,
    /// Go's GC CPU limiter: suppresses assists when the GC uses too much CPU.
    pub(super) limiter: CpuLimiter,
    /// When the current pause started, on the pacer's clock.
    pause_start: AtomicI64,
    /// Go's `GOMAXPROCS`: the CPUs shared by the application and the GC.
    procs: usize,
    /// The last `heap_live` reported to the pacer, in bytes.
    reported_heap_live: AtomicU64,
    /// The origin of the pacer's clock.
    epoch: Instant,
    /// Root scan work of the current cycle, in bytes, and the values reported to the pacer as
    /// the scannable stack and globals size (Go's `maxStackScan` and `globalsScan`).
    stack_scan: AtomicU64,
    globals_scan: AtomicU64,
    reported_stack: AtomicU64,
    reported_globals: AtomicU64,
}

impl Pacing {
    pub(super) fn new(options: &Options) -> Self {
        let gc_percent = match *options.gc_percent {
            -1 => None,
            p => Some(p as u32),
        };
        let mem_limit = match *options.soft_mem_limit {
            0 => options.gc_trigger.max_heap_size(),
            limit => limit,
        };
        let procs = num_cpus::get();
        Self {
            pacer: FeedbackPacer::new(gc_percent, Some(mem_limit as u64)),
            limiter: CpuLimiter::new(0, procs as i32),
            pause_start: AtomicI64::new(0),
            procs: num_cpus::get(),
            reported_heap_live: AtomicU64::new(0),
            epoch: Instant::now(),
            stack_scan: AtomicU64::new(0),
            globals_scan: AtomicU64::new(0),
            reported_stack: AtomicU64::new(0),
            reported_globals: AtomicU64::new(0),
        }
    }

    fn now(&self) -> i64 {
        self.epoch.elapsed().as_nanos() as i64
    }

    /// Tell the pacer the current size of the live heap estimate (Go's `heapLive`), in bytes.
    /// Every object is scannable on the VMs we target, so the change is also added to
    /// `heap_scan`.
    pub(super) fn report_heap_live(&self, heap_live: u64) {
        let old = self.reported_heap_live.swap(heap_live, Ordering::Relaxed);
        let delta = heap_live as i64 - old as i64;
        if delta != 0 {
            self.pacer.update(delta, delta);
        }
    }

    /// Return `true` if the heap has grown past the pacer's trigger.
    pub(super) fn should_start_cycle(&self) -> bool {
        let (trigger, _goal) = self.pacer.trigger();
        self.pacer.heap_live() >= trigger
    }

    /// Report scan work done while marking, in bytes.
    pub(super) fn add_scan_work(&self, heap: usize, stack: usize, globals: usize) {
        self.pacer
            .add_scan_work(heap as i64, stack as i64, globals as i64);
        if stack != 0 {
            self.stack_scan.fetch_add(stack as u64, Ordering::Relaxed);
        }
        if globals != 0 {
            self.globals_scan
                .fetch_add(globals as u64, Ordering::Relaxed);
        }
    }

    pub(super) fn now_ns(&self) -> i64 {
        self.now()
    }

    /// A pause started.
    pub(super) fn pause_started(&self) {
        self.pause_start.store(self.now(), Ordering::Relaxed);
    }

    /// A pause that turned marking on or off has ended (Go: `gcCPULimiter.startGCTransition`
    /// and `finishGCTransition` around the STW pauses).  The pause counts as GC time on every
    /// CPU.
    pub(super) fn limiter_transition(&self, marking: bool) {
        let start = self.pause_start.load(Ordering::Relaxed);
        self.limiter.start_gc_transition(marking, start);
        self.limiter.finish_gc_transition(self.now());
    }

    /// Return `true` if the CPU limiter is limiting, so assists should not run (Go:
    /// `gcCPULimiter.limiting()` in `gcAssistAlloc`).  Updates the limiter if it is due.
    pub(super) fn limiting(&self) -> bool {
        let now = self.now();
        if self.limiter.need_update(now) {
            self.limiter.update(now);
        }
        self.limiter.limiting()
    }

    /// A cycle starts (Go: `gcStart`).
    pub(super) fn start_cycle(&self) {
        self.stack_scan.store(0, Ordering::Relaxed);
        self.globals_scan.store(0, Ordering::Relaxed);
        self.pacer.start_cycle(self.now(), self.procs, false);
        self.pacer.set_blacken_enabled(true);
    }

    /// Marking is done (Go: `gcMarkDone`).
    pub(super) fn end_marking(&self) {
        // Go keeps the scannable stack size up to date as stacks grow, and the globals size is
        // the size of the data and BSS segments.  Here both are what this cycle found.
        let stack = self.stack_scan.load(Ordering::Relaxed);
        let old = self.reported_stack.swap(stack, Ordering::Relaxed);
        self.pacer.add_scannable_stack(stack as i64 - old as i64);
        let globals = self.globals_scan.load(Ordering::Relaxed);
        let old = self.reported_globals.swap(globals, Ordering::Relaxed);
        self.pacer.add_globals(globals as i64 - old as i64);
        self.pacer.set_blacken_enabled(false);
        self.pacer.end_cycle(self.now(), self.procs);
    }

    /// The cycle ends (Go: `gcMarkTermination`).  `heap_marked` is the size of the live heap
    /// found by marking, in bytes.
    pub(super) fn end_cycle(&self, heap_marked: u64) {
        self.pacer.reset_live(heap_marked);
        self.reported_heap_live
            .store(heap_marked, Ordering::Relaxed);
        let (trigger, goal) = self.pacer.controller_commit(true, true);
        info!(
            "Pacer: heap marked {} bytes, next trigger {} bytes, heap goal {} bytes",
            heap_marked, trigger, goal
        );
    }
}
