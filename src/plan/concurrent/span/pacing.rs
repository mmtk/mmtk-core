//! Connects [`FeedbackPacer`] to the plan: what the pacer is told, and when.
//!
//! The pacer decides when a cycle starts.  The heap size from the GC trigger (for example
//! `FixedHeapSize`, which is `-Xmx` on OpenJDK) stays the hard limit, and is also the pacer's
//! memory limit unless `soft_mem_limit` is set.

use super::pacer::FeedbackPacer;
use crate::util::options::Options;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub(super) struct Pacing {
    pub(super) pacer: FeedbackPacer,
    /// Go's `GOMAXPROCS`: the CPUs shared by the application and the GC.
    procs: usize,
    /// The last `heap_live` reported to the pacer, in bytes.
    reported_heap_live: AtomicU64,
    /// The origin of the pacer's clock.
    epoch: Instant,
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
        Self {
            pacer: FeedbackPacer::new(gc_percent, Some(mem_limit as u64)),
            procs: num_cpus::get(),
            reported_heap_live: AtomicU64::new(0),
            epoch: Instant::now(),
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

    /// A cycle starts (Go: `gcStart`).
    pub(super) fn start_cycle(&self) {
        self.pacer.start_cycle(self.now(), self.procs, false);
        self.pacer.set_blacken_enabled(true);
    }

    /// Marking is done (Go: `gcMarkDone`).
    pub(super) fn end_marking(&self) {
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
