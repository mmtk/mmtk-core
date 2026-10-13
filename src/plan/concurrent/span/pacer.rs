// This file is derived from the Go runtime's garbage collection pacer,
// `runtime/mgcpacer.go` at Go commit 2c3c1caad2. It is a port of Go's code,
// not a product of the Go project, and implies no endorsement by Google or the
// Go contributors. The original copyright notice and licence follow.
//
// Copyright 2021 The Go Authors. All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
//    * Redistributions of source code must retain the above copyright
// notice, this list of conditions and the following disclaimer.
//    * Redistributions in binary form must reproduce the above
// copyright notice, this list of conditions and the following disclaimer
// in the documentation and/or other materials provided with the
// distribution.
//    * Neither the name of Google LLC nor the names of its
// contributors may be used to endorse or promote products derived from
// this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! `FeedbackPacer`: a port of Go's GC pacer (`gcControllerState` in
//! `runtime/mgcpacer.go`, design 44167).
//!
//! The pacer decides when a concurrent cycle starts (the *trigger*), the heap
//! size at which it should end (the *heap goal*), and how much scan work a
//! mutator must do per byte it allocates during marking (the *assist ratio*).
//! See `docs/design/go-gc-port.md` §3.6 and §5.10.
//!
//! This module is pure arithmetic and depends only on `std`, so it is
//! unit-tested directly against Go's `TestGcPacer` simulation (see `tests.rs`).
//! Functions keep the structure of the Go originals, and each cites the Go
//! source it ports.
//!
//! # Units
//!
//! As in Go:
//! - heap sizes, scan work, stack and globals sizes are in bytes;
//! - times are in nanoseconds, from any monotonic clock (only differences are
//!   used);
//! - `procs` is the number of CPUs that the application and the GC share (Go's
//!   `GOMAXPROCS`).
//!
//! # Concurrency
//!
//! Go keeps the pacer state in a mix of atomics and fields that are only written
//! with the world stopped. Here every field is an atomic and every method takes
//! `&self`, so the pacer can be shared (for example inside a `GCTriggerPolicy`).
//! All atomics use `Relaxed` ordering: the pacer's inputs are heuristics, and Go
//! itself tolerates the races (see the comment on [`FeedbackPacer::revise`]).
//! Methods that Go only calls with the world stopped say so in their docs; the
//! caller must provide that exclusion.
//!
//! # Cycle protocol
//!
//! The order of calls in one GC cycle, following `runtime/mgc.go`:
//!
//! 1. At the start of the cycle (Go: `gcStart`, in the sweep-termination pause):
//!    [`FeedbackPacer::start_cycle`], then [`FeedbackPacer::set_blacken_enabled`]`(true)`.
//! 2. During marking: mutators report heap growth through
//!    [`FeedbackPacer::update`] (which calls `revise`), workers report scan work
//!    through [`FeedbackPacer::add_scan_work`], and assists read
//!    [`FeedbackPacer::assist_work_per_byte`].
//! 3. When marking is done (Go: `gcMarkDone`): [`FeedbackPacer::set_blacken_enabled`]`(false)`,
//!    then [`FeedbackPacer::end_cycle`].
//! 4. In the mark-termination pause: [`FeedbackPacer::reset_live`] with the bytes
//!    marked, then [`FeedbackPacer::controller_commit`].
//! 5. Between cycles, the allocator compares [`FeedbackPacer::heap_live`] with
//!    the trigger from [`FeedbackPacer::trigger`].

#![allow(clippy::too_many_arguments)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};

#[cfg(test)]
mod tests;

const ORD: Ordering = Ordering::Relaxed;

// Go: runtime/mgcpacer.go:16-76 (2c3c1caad2)

/// The goal CPU utilisation for marking as a fraction of `procs`.
// Go: runtime/mgcpacer.go:23 (2c3c1caad2)
pub const GC_GOAL_UTILIZATION: f64 = GC_BACKGROUND_UTILIZATION;

/// The fixed CPU utilisation for background marking. It must be
/// `<= GC_GOAL_UTILIZATION`. The difference is made up by mutator assists.
// Go: runtime/mgcpacer.go:39 (2c3c1caad2)
pub const GC_BACKGROUND_UTILIZATION: f64 = 0.25;

/// The amount of scan work credit that can accumulate locally before updating
/// the pacer's scan work counters and the background scan credit.
// Go: runtime/mgcpacer.go:47 (2c3c1caad2)
pub const GC_CREDIT_SLACK: i64 = 2000;

/// The nanoseconds of mutator assist time that can accumulate locally before
/// updating the pacer's assist time.
// Go: runtime/mgcpacer.go:51 (2c3c1caad2)
pub const GC_ASSIST_TIME_SLACK: i64 = 5000;

/// How many extra units of scan work a GC assist does when an assist happens,
/// to amortise its cost.
// Go: runtime/mgcpacer.go:56 (2c3c1caad2)
pub const GC_OVER_ASSIST_WORK: i64 = 64 << 10;

/// The value of `heap_minimum` for `gc_percent == 100`.
///
/// Go selects 512 KiB instead when the `HeapMinimum512KiB` experiment is on.
/// The experiment is off by default, so this is 4 MiB (decision D20).
// Go: runtime/mgcpacer.go:59-60 (2c3c1caad2)
pub const DEFAULT_HEAP_MINIMUM: u64 = 4 << 20;

/// The bytes of stack space allocated or freed that can accumulate locally
/// before updating `max_stack_scan`.
// Go: runtime/mgcpacer.go:64 (2c3c1caad2)
pub const MAX_STACK_SCAN_SLACK: i64 = 8 << 10;

/// The minimum headroom the pacer gives to the heap goal in the memory-limited
/// regime.
// Go: runtime/mgcpacer.go:70 (2c3c1caad2)
pub const MEMORY_LIMIT_MIN_HEAP_GOAL_HEADROOM: u64 = 1 << 20;

/// The headroom of the memory-limit-based heap goal, as a percentage of the
/// maximum possible heap goal that maintains the memory limit.
// Go: runtime/mgcpacer.go:75 (2c3c1caad2)
pub const MEMORY_LIMIT_HEAP_GOAL_HEADROOM_PERCENT: u64 = 3;

/// The minimum distance, in bytes of heap growth, given to the concurrent
/// sweeper before the next trigger.
// Go: runtime/mgc.go:154 (2c3c1caad2)
pub const SWEEP_MIN_HEAP_DISTANCE: u64 = 1024 * 1024;

// The bounds on the trigger as a fraction (`num / TRIGGER_RATIO_DEN`) of the
// heap growth between heapMarked and the heap goal.
// Go: runtime/mgcpacer.go:1160-1179 (2c3c1caad2)
const TRIGGER_RATIO_DEN: u64 = 64;
const MIN_TRIGGER_RATIO_NUM: u64 = 45; // ~0.7
const MAX_TRIGGER_RATIO_NUM: u64 = 61; // ~0.95

/// An `f64` stored in an `AtomicU64` (Go's `atomic.Float64`).
#[derive(Debug, Default)]
struct AtomicF64(AtomicU64);

impl AtomicF64 {
    const fn zero() -> Self {
        // 0.0f64 has the bit pattern 0.
        AtomicF64(AtomicU64::new(0))
    }
    fn load(&self) -> f64 {
        f64::from_bits(self.0.load(ORD))
    }
    fn store(&self, v: f64) {
        self.0.store(v.to_bits(), ORD)
    }
}

/// The kind of a background mark worker, for [`FeedbackPacer::mark_worker_stop`].
// Go: runtime/mgc.go gcMarkWorkerMode (2c3c1caad2)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkWorkerMode {
    /// A worker that runs on a CPU dedicated to marking.
    Dedicated,
    /// A worker that runs part-time to make up the fractional utilisation goal.
    Fractional,
    /// A worker that only uses otherwise idle CPU.
    Idle,
}

/// Go's GC pacer (`gcControllerState`). See the module documentation.
// Go: runtime/mgcpacer.go:92-369 (2c3c1caad2)
#[derive(Debug)]
pub struct FeedbackPacer {
    /// Initialised from `gc_percent`. `-1` (off) means no GC.
    gc_percent: AtomicI32,

    /// The soft memory limit in bytes. `i64::MAX` means no limit in practice.
    memory_limit: AtomicI64,

    /// The minimum heap goal. For small heaps, this overrides the usual
    /// `gc_percent` rule. Set to `DEFAULT_HEAP_MINIMUM * gc_percent / 100`.
    heap_minimum: AtomicU64,

    /// The heap bytes the application may allocate once a cycle starts,
    /// computed from `cons_mark` in `commit`.
    runway: AtomicU64,

    /// The estimated ratio between the allocation rate and the scan rate, both
    /// per CPU-time. Updated in `end_cycle`.
    cons_mark: AtomicF64,

    /// The measured cons/mark values of the last 4 cycles.
    last_cons_mark: [AtomicF64; 4],

    /// The heap goal derived from `gc_percent`. `u64::MAX` if `gc_percent` is
    /// off.
    gc_percent_heap_goal: AtomicU64,

    /// The minimum trigger that ensures a minimum sweep distance. Computed in
    /// `commit`; applies to both the trigger and the goal.
    sweep_dist_min_trigger: AtomicU64,

    /// The `heap_live` at which the current cycle triggered. Only valid during
    /// marking, otherwise `u64::MAX`.
    triggered: AtomicU64,

    /// The heap goal at the moment the last cycle ended.
    last_heap_goal: AtomicU64,

    /// The bytes considered live by the GC: retained by the last cycle plus
    /// allocated since then. Overestimates (counts whole spans at refill).
    heap_live: AtomicU64,

    /// Bytes of scannable heap. Fixed at the start of a cycle.
    heap_scan: AtomicU64,

    /// Bytes of heap scanned in the last cycle.
    last_heap_scan: AtomicU64,

    /// Bytes of stack scanned in the last cycle.
    last_stack_scan: AtomicU64,

    /// Allocated (not used) stack space, an overestimate of stack scan work.
    max_stack_scan: AtomicU64,

    /// Total scannable globals (roots outside the heap and stacks).
    globals_scan: AtomicU64,

    /// Bytes marked by the previous cycle.
    heap_marked: AtomicU64,

    /// Heap, stack and globals scan work performed this cycle.
    heap_scan_work: AtomicI64,
    stack_scan_work: AtomicI64,
    globals_scan_work: AtomicI64,

    /// Scan work credit accumulated by background marking, stolen by assists.
    bg_scan_credit: AtomicI64,

    /// Nanoseconds spent in mutator assists this cycle.
    assist_time: AtomicI64,

    /// Nanoseconds spent in dedicated, fractional and idle mark workers this
    /// cycle.
    dedicated_mark_time: AtomicI64,
    fractional_mark_time: AtomicI64,
    idle_mark_time: AtomicI64,

    /// The time at which assists and background mark workers started.
    mark_start_time: AtomicI64,

    /// The number of dedicated mark workers that still need to be started.
    dedicated_mark_workers_needed: AtomicI64,

    /// Scan work that assists should do per byte allocated.
    assist_work_per_byte: AtomicF64,

    /// `1 / assist_work_per_byte`.
    assist_bytes_per_work: AtomicF64,

    /// The fraction of wall-clock time each CPU that does not run a dedicated
    /// worker should spend in a fractional worker. Zero if none is needed.
    fractional_utilization_goal: AtomicF64,

    /// Memory statistics for the memory-limit goal (Go: `heapFree`,
    /// `totalAlloc - totalFree`, `mappedReady`). See
    /// [`FeedbackPacer::set_memory_stats`].
    heap_free: AtomicU64,
    heap_alloc: AtomicU64,
    mapped_ready: AtomicU64,

    /// Go's `gcBlackenEnabled`: whether marking (and so assists) is running.
    blacken_enabled: AtomicBool,
}

impl FeedbackPacer {
    /// Create a pacer.
    ///
    /// * `gc_percent`: the heap growth allowed over the live heap, in percent
    ///   (Go's `GOGC`). `None` turns the `gc_percent` goal off (`GOGC=off`), so
    ///   only the memory limit triggers collections. Values above `i32::MAX`
    ///   are clamped.
    /// * `mem_limit`: the soft memory limit in bytes (Go's `GOMEMLIMIT`).
    ///   `None` means no limit. Values above `i64::MAX` are clamped.
    // Go: runtime/mgcpacer.go:371-381 (2c3c1caad2)
    pub fn new(gc_percent: Option<u32>, mem_limit: Option<u64>) -> Self {
        Self::new_raw(
            Self::gc_percent_to_raw(gc_percent),
            Self::memory_limit_to_raw(mem_limit),
        )
    }

    /// Create a pacer from Go's raw values: `gc_percent < 0` means off,
    /// `memory_limit == i64::MAX` means no limit.
    // Go: runtime/mgcpacer.go:371-381 (2c3c1caad2)
    pub(crate) fn new_raw(gc_percent: i32, memory_limit: i64) -> Self {
        let c = FeedbackPacer {
            gc_percent: AtomicI32::new(0),
            memory_limit: AtomicI64::new(0),
            heap_minimum: AtomicU64::new(0),
            runway: AtomicU64::new(0),
            cons_mark: AtomicF64::zero(),
            last_cons_mark: [
                AtomicF64::zero(),
                AtomicF64::zero(),
                AtomicF64::zero(),
                AtomicF64::zero(),
            ],
            gc_percent_heap_goal: AtomicU64::new(0),
            sweep_dist_min_trigger: AtomicU64::new(0),
            triggered: AtomicU64::new(0),
            last_heap_goal: AtomicU64::new(0),
            heap_live: AtomicU64::new(0),
            heap_scan: AtomicU64::new(0),
            last_heap_scan: AtomicU64::new(0),
            last_stack_scan: AtomicU64::new(0),
            max_stack_scan: AtomicU64::new(0),
            globals_scan: AtomicU64::new(0),
            heap_marked: AtomicU64::new(0),
            heap_scan_work: AtomicI64::new(0),
            stack_scan_work: AtomicI64::new(0),
            globals_scan_work: AtomicI64::new(0),
            bg_scan_credit: AtomicI64::new(0),
            assist_time: AtomicI64::new(0),
            dedicated_mark_time: AtomicI64::new(0),
            fractional_mark_time: AtomicI64::new(0),
            idle_mark_time: AtomicI64::new(0),
            mark_start_time: AtomicI64::new(0),
            dedicated_mark_workers_needed: AtomicI64::new(0),
            assist_work_per_byte: AtomicF64::zero(),
            assist_bytes_per_work: AtomicF64::zero(),
            fractional_utilization_goal: AtomicF64::zero(),
            heap_free: AtomicU64::new(0),
            heap_alloc: AtomicU64::new(0),
            mapped_ready: AtomicU64::new(0),
            blacken_enabled: AtomicBool::new(false),
        };
        c.heap_minimum.store(DEFAULT_HEAP_MINIMUM, ORD);
        c.triggered.store(u64::MAX, ORD);
        c.set_gc_percent_raw(gc_percent);
        c.set_memory_limit_raw(memory_limit);
        c.commit(true); // No sweep phase in the first GC cycle.
        c
    }

    fn gc_percent_to_raw(gc_percent: Option<u32>) -> i32 {
        match gc_percent {
            None => -1,
            Some(p) => i32::try_from(p).unwrap_or(i32::MAX),
        }
    }

    fn memory_limit_to_raw(mem_limit: Option<u64>) -> i64 {
        match mem_limit {
            None => i64::MAX,
            Some(l) => i64::try_from(l).unwrap_or(i64::MAX),
        }
    }

    /// Reset the pacer's per-cycle state and compute the estimates for a new
    /// cycle. Call with the world stopped, at the start of a cycle.
    ///
    /// * `mark_start_time`: the time (ns) at which assists and background
    ///   workers start. `end_cycle`'s `now` is measured against it.
    /// * `procs`: the number of CPUs shared by the application and the GC (Go's
    ///   `GOMAXPROCS`). It sets the number of dedicated workers
    ///   (`round(0.25 * procs)`) and the fractional goal, and must be the same
    ///   value passed to `end_cycle`.
    /// * `periodic`: whether the cycle was started by a timer rather than heap
    ///   growth (Go's `gcTriggerTime`). Go uses it to limit idle mark workers,
    ///   which are not ported, so it is currently unused.
    ///
    /// `heap_live`, `heap_scan`, `max_stack_scan` and `globals_scan` must be up
    /// to date. `heap_live` is recorded as the trigger point.
    // Go: runtime/mgcpacer.go:383-468 (2c3c1caad2)
    pub fn start_cycle(&self, mark_start_time: i64, procs: usize, periodic: bool) {
        self.heap_scan_work.store(0, ORD);
        self.stack_scan_work.store(0, ORD);
        self.globals_scan_work.store(0, ORD);
        self.bg_scan_credit.store(0, ORD);
        self.assist_time.store(0, ORD);
        self.dedicated_mark_time.store(0, ORD);
        self.fractional_mark_time.store(0, ORD);
        self.idle_mark_time.store(0, ORD);
        self.mark_start_time.store(mark_start_time, ORD);
        self.triggered.store(self.heap_live.load(ORD), ORD);

        // Compute the background mark utilization goal. In general,
        // this may not come out exactly. We round the number of
        // dedicated workers so that the utilization is closest to
        // 25%. For small GOMAXPROCS, this would introduce too much
        // error, so we add fractional workers in that case.
        let total_utilization_goal = procs as f64 * GC_BACKGROUND_UTILIZATION;
        let mut dedicated_mark_workers_needed = (total_utilization_goal + 0.5) as i64;
        let util_error = dedicated_mark_workers_needed as f64 / total_utilization_goal - 1.0;
        const MAX_UTIL_ERROR: f64 = 0.3;
        if !(-MAX_UTIL_ERROR..=MAX_UTIL_ERROR).contains(&util_error) {
            // Rounding put us more than 30% off our goal. With
            // gcBackgroundUtilization of 25%, this happens for
            // GOMAXPROCS<=3 or GOMAXPROCS=6. Enable fractional
            // workers to compensate.
            if dedicated_mark_workers_needed as f64 > total_utilization_goal {
                // Too many dedicated workers.
                dedicated_mark_workers_needed -= 1;
            }
            self.fractional_utilization_goal.store(
                (total_utilization_goal - dedicated_mark_workers_needed as f64) / procs as f64,
            );
        } else {
            self.fractional_utilization_goal.store(0.0);
        }

        // Go: STW mode (debug.gcstoptheworld) and clearing per-P state are not
        // ported. See the deviations document.

        // Go: the maximum number of idle mark workers is set here
        // (runtime/mgcpacer.go:433-451). Idle mark workers are not ported, so
        // `periodic` has no effect. See the deviations document.
        let _ = periodic;

        // Compute initial values for controls that are updated
        // throughout the cycle.
        self.dedicated_mark_workers_needed
            .store(dedicated_mark_workers_needed, ORD);
        self.revise();
    }

    /// Update the assist ratio during the cycle to account for improved
    /// estimates. Call whenever `heap_scan`, `heap_live` or any input to the
    /// heap goal changes, while marking is enabled.
    ///
    /// It is safe to call concurrently, but it may race with other calls. As in
    /// Go, the two assist ratio values may then be stale or not exact
    /// reciprocals; the assist ratio moves slowly and is best-effort anyway.
    // Go: runtime/mgcpacer.go:470-595 (2c3c1caad2)
    pub fn revise(&self) {
        let mut gc_percent = self.gc_percent.load(ORD);
        if gc_percent < 0 {
            // If GC is disabled but we're running a forced GC,
            // act like GOGC is huge for the below calculations.
            gc_percent = 100000;
        }
        let live = self.heap_live.load(ORD);
        let scan = self.heap_scan.load(ORD);
        let work = self
            .heap_scan_work
            .load(ORD)
            .wrapping_add(self.stack_scan_work.load(ORD))
            .wrapping_add(self.globals_scan_work.load(ORD));

        // Assume we're under the soft goal. Pace GC to complete at
        // heapGoal assuming the heap is in steady-state.
        let mut heap_goal = self.heap_goal() as i64;

        // The expected scan work is computed as the amount of bytes scanned last
        // GC cycle (both heap and stack), plus our estimate of globals work for this cycle.
        let mut scan_work_expected = self
            .last_heap_scan
            .load(ORD)
            .wrapping_add(self.last_stack_scan.load(ORD))
            .wrapping_add(self.globals_scan.load(ORD)) as i64;

        // maxScanWork is a worst-case estimate of the amount of scan work that
        // needs to be performed in this GC cycle. Specifically, it represents
        // the case where *all* scannable memory turns out to be live, and
        // *all* allocated stack space is scannable.
        let max_stack_scan = self.max_stack_scan.load(ORD);
        let max_scan_work = scan
            .wrapping_add(max_stack_scan)
            .wrapping_add(self.globals_scan.load(ORD)) as i64;
        if work > scan_work_expected {
            // We've already done more scan work than expected. Because our expectation
            // is based on a steady-state scannable heap size, we assume this means our
            // heap is growing. Compute a new heap goal that takes our existing runway
            // computed for scanWorkExpected and extrapolates it to maxScanWork, the worst-case
            // scan work. This keeps our assist ratio stable if the heap continues to grow.
            //
            // The effect of this mechanism is that assists stay flat in the face of heap
            // growths. It's OK to use more memory this cycle to scan all the live heap,
            // because the next GC cycle is inevitably going to use *at least* that much
            // memory anyway.
            let triggered = self.triggered.load(ORD) as i64;
            let mut ext_heap_goal = ((heap_goal.wrapping_sub(triggered) as f64
                / scan_work_expected as f64
                * max_scan_work as f64) as i64)
                .wrapping_add(triggered);
            scan_work_expected = max_scan_work;

            // hardGoal is a hard limit on the amount that we're willing to push back the
            // heap goal, and that's twice the heap goal (i.e. if GOGC=100 and the heap and/or
            // stacks and/or globals grow to twice their size, this limits the current GC cycle's
            // growth to 4x the original live heap's size).
            //
            // This maintains the invariant that we use no more memory than the next GC cycle
            // will anyway.
            let hard_goal = ((1.0 + gc_percent as f64 / 100.0) * heap_goal as f64) as i64;
            if ext_heap_goal > hard_goal {
                ext_heap_goal = hard_goal;
            }
            heap_goal = ext_heap_goal;
        }
        if live as i64 > heap_goal {
            // We're already past our heap goal, even the extrapolated one.
            // Leave ourselves some extra runway, so in the worst case we
            // finish by that point.
            const MAX_OVERSHOOT: f64 = 1.1;
            heap_goal = (heap_goal as f64 * MAX_OVERSHOOT) as i64;

            // Compute the upper bound on the scan work remaining.
            scan_work_expected = max_scan_work;
        }

        // Compute the remaining scan work estimate.
        //
        // Note that we currently count allocations during GC as both
        // scannable heap (heapScan) and scan work completed
        // (scanWork), so allocation will change this difference
        // slowly in the soft regime and not at all in the hard
        // regime.
        let mut scan_work_remaining = scan_work_expected.wrapping_sub(work);
        if scan_work_remaining < 1000 {
            // We set a somewhat arbitrary lower bound on
            // remaining scan work since if we aim a little high,
            // we can miss by a little.
            //
            // We *do* need to enforce that this is at least 1,
            // since marking is racy and double-scanning objects
            // may legitimately make the remaining scan work
            // negative, even in the hard goal regime.
            scan_work_remaining = 1000;
        }

        // Compute the heap distance remaining.
        let mut heap_remaining = heap_goal.wrapping_sub(live as i64);
        if heap_remaining <= 0 {
            // This shouldn't happen, but if it does, avoid
            // dividing by zero or setting the assist negative.
            heap_remaining = 1;
        }

        // Compute the mutator assist ratio so by the time the mutator
        // allocates the remaining heap bytes up to heapGoal, it will
        // have done (or stolen) the remaining amount of scan work.
        let assist_work_per_byte = scan_work_remaining as f64 / heap_remaining as f64;
        let assist_bytes_per_work = heap_remaining as f64 / scan_work_remaining as f64;
        self.assist_work_per_byte.store(assist_work_per_byte);
        self.assist_bytes_per_work.store(assist_bytes_per_work);
    }

    /// Compute the cons/mark estimate for the next cycle. Call once when
    /// marking is done, after the last scan work and assist time have been
    /// reported and before `reset_live`.
    ///
    /// * `now`: the current time (ns), on the same clock as `start_cycle`'s
    ///   `mark_start_time`.
    /// * `procs`: the same value passed to `start_cycle`.
    // Go: runtime/mgcpacer.go:597-684 (2c3c1caad2)
    pub fn end_cycle(&self, now: i64, procs: usize) {
        // Record last heap goal for the scavenger.
        // We'll be updating the heap goal soon.
        self.last_heap_goal.store(self.heap_goal(), ORD);

        // Compute the duration of time for which assists were turned on.
        let assist_duration = now.wrapping_sub(self.mark_start_time.load(ORD));

        // Assume background mark hit its utilization goal.
        let mut utilization = GC_BACKGROUND_UTILIZATION;
        // Add assist utilization; avoid divide by zero.
        if assist_duration > 0 {
            utilization += self.assist_time.load(ORD) as f64
                / assist_duration.wrapping_mul(procs as i64) as f64;
        }

        let heap_live = self.heap_live.load(ORD);
        let triggered = self.triggered.load(ORD);
        if heap_live <= triggered {
            // Shouldn't happen, but let's be very safe about this in case the
            // GC is somehow extremely short.
            //
            // In this case though, the only reasonable value for c.heapLive-c.triggered
            // would be 0, which isn't really all that useful, i.e. the GC was so short
            // that it didn't matter.
            //
            // Ignore this case and don't update anything.
            return;
        }
        let mut idle_utilization = 0.0;
        if assist_duration > 0 {
            idle_utilization = self.idle_mark_time.load(ORD) as f64
                / assist_duration.wrapping_mul(procs as i64) as f64;
        }
        // Determine the cons/mark ratio.
        //
        // The units we want for the numerator and denominator are both B / cpu-ns.
        // We get this by taking the bytes allocated or scanned, and divide by the amount of
        // CPU time it took for those operations. For allocations, that CPU time is
        //
        //    assistDuration * procs * (1 - utilization)
        //
        // Where utilization includes just background GC workers and assists. It does *not*
        // include idle GC work time, because in theory the mutator is free to take that at
        // any point.
        //
        // For scanning, that CPU time is
        //
        //    assistDuration * procs * (utilization + idleUtilization)
        //
        // In this case, we *include* idle utilization, because that is additional CPU time that
        // the GC had available to it.
        //
        // In effect, idle GC time is sort of double-counted here, but it's very weird compared
        // to other kinds of GC work, because of how fluid it is. Namely, because the mutator is
        // *always* free to take it.
        //
        // So this calculation is really:
        //     (heapLive-trigger) / (assistDuration * procs * (1-utilization)) /
        //         (scanWork) / (assistDuration * procs * (utilization+idleUtilization))
        //
        // Note that because we only care about the ratio, assistDuration and procs cancel out.
        let scan_work = self
            .heap_scan_work
            .load(ORD)
            .wrapping_add(self.stack_scan_work.load(ORD))
            .wrapping_add(self.globals_scan_work.load(ORD));
        let current_cons_mark = ((heap_live - triggered) as f64 * (utilization + idle_utilization))
            / (scan_work as f64 * (1.0 - utilization));

        // Update our cons/mark estimate. This is the maximum of the value we just computed and the last
        // 4 cons/mark values we measured. The reason we take the maximum here is to bias a noisy
        // cons/mark measurement toward fewer assists at the expense of additional GC cycles (starting
        // earlier).
        let mut cons_mark = current_cons_mark;
        for last in self.last_cons_mark.iter() {
            let last = last.load();
            if last > cons_mark {
                cons_mark = last;
            }
        }
        self.cons_mark.store(cons_mark);
        let n = self.last_cons_mark.len();
        for i in 0..n - 1 {
            self.last_cons_mark[i].store(self.last_cons_mark[i + 1].load());
        }
        self.last_cons_mark[n - 1].store(current_cons_mark);

        // Go: the gcpacertrace debug output is not ported.
    }

    /// Set up the pacer for the next mark phase after the end of the previous
    /// one. Call with the world stopped, after `end_cycle` and before `commit`.
    ///
    /// * `bytes_marked`: the bytes marked by the cycle that just ended,
    ///   including objects allocated black during marking.
    // Go: runtime/mgcpacer.go:910-929 (2c3c1caad2)
    pub fn reset_live(&self, bytes_marked: u64) {
        self.heap_marked.store(bytes_marked, ORD);
        self.heap_live.store(bytes_marked, ORD);
        let heap_scan_work = self.heap_scan_work.load(ORD) as u64;
        self.heap_scan.store(heap_scan_work, ORD);
        self.last_heap_scan.store(heap_scan_work, ORD);
        self.last_stack_scan
            .store(self.stack_scan_work.load(ORD) as u64, ORD);
        self.triggered.store(u64::MAX, ORD); // Reset triggered.
    }

    /// Account for a mark worker that stopped executing after running for
    /// `duration` ns.
    ///
    /// A stopped dedicated worker makes its slot available again
    /// (`dedicated_mark_workers_needed` is incremented). Idle mark time counts
    /// towards the GC's CPU time in `end_cycle`.
    // Go: runtime/mgcpacer.go:931-950 (2c3c1caad2)
    pub fn mark_worker_stop(&self, mode: MarkWorkerMode, duration: i64) {
        match mode {
            MarkWorkerMode::Dedicated => {
                self.dedicated_mark_time.fetch_add(duration, ORD);
                self.dedicated_mark_workers_needed.fetch_add(1, ORD);
            }
            MarkWorkerMode::Fractional => {
                self.fractional_mark_time.fetch_add(duration, ORD);
            }
            MarkWorkerMode::Idle => {
                // Go also removes the idle worker from the idle worker count,
                // which is not ported.
                self.idle_mark_time.fetch_add(duration, ORD);
            }
        }
    }

    /// Report a change in `heap_live` and `heap_scan`.
    ///
    /// * `d_heap_live`: bytes added to (or, if negative, removed from) the live
    ///   heap estimate. Go charges a whole span's free slots when a cache takes
    ///   the span, and refunds the unused part when it returns the span.
    /// * `d_heap_scan`: the scannable part of that change. It is only applied
    ///   between cycles, because `heap_scan` is fixed while marking.
    ///
    /// While marking is enabled this calls `revise`.
    // Go: runtime/mgcpacer.go:952-972 (2c3c1caad2)
    pub fn update(&self, d_heap_live: i64, d_heap_scan: i64) {
        if d_heap_live != 0 {
            self.heap_live.fetch_add(d_heap_live as u64, ORD);
        }
        if !self.blacken_enabled.load(ORD) {
            // Update heapScan when we're not in a current GC. It is fixed
            // at the beginning of a cycle.
            if d_heap_scan != 0 {
                self.heap_scan.fetch_add(d_heap_scan as u64, ORD);
            }
        } else {
            // gcController.heapLive changed.
            self.revise();
        }
    }

    /// Report a change of `amount` bytes in allocated stack space (an
    /// overestimate of the stack scan work). Go batches this per P with
    /// `MAX_STACK_SCAN_SLACK`; callers may do the same.
    // Go: runtime/mgcpacer.go:974-984 (2c3c1caad2), the `pp == nil` path
    pub fn add_scannable_stack(&self, amount: i64) {
        self.max_stack_scan.fetch_add(amount as u64, ORD);
    }

    /// Report a change of `amount` bytes in scannable globals (roots that are
    /// neither in the heap nor on stacks).
    // Go: runtime/mgcpacer.go:986-988 (2c3c1caad2)
    pub fn add_globals(&self, amount: i64) {
        self.globals_scan.fetch_add(amount as u64, ORD);
    }

    /// Report scan work (bytes scanned) done during this cycle, split into heap,
    /// stack and globals. Go flushes these in batches of `GC_CREDIT_SLACK`.
    // Go: runtime/mgcmark.go (gcw.heapScanWork flushes, scanstack, markroot)
    pub fn add_scan_work(&self, heap: i64, stack: i64, globals: i64) {
        if heap != 0 {
            self.heap_scan_work.fetch_add(heap, ORD);
        }
        if stack != 0 {
            self.stack_scan_work.fetch_add(stack, ORD);
        }
        if globals != 0 {
            self.globals_scan_work.fetch_add(globals, ORD);
        }
    }

    /// Report `ns` nanoseconds of mutator assist time this cycle. Go batches
    /// this per P with `GC_ASSIST_TIME_SLACK`.
    // Go: runtime/mgcmark.go gcAssistAlloc1 (assistTime.Add)
    pub fn add_assist_time(&self, ns: i64) {
        self.assist_time.fetch_add(ns, ORD);
    }

    /// Set Go's `gcBlackenEnabled`: true while marking (and assists) run. It
    /// decides whether `update` revises the assist ratio or updates `heap_scan`.
    pub fn set_blacken_enabled(&self, enabled: bool) {
        self.blacken_enabled.store(enabled, ORD);
    }

    /// Whether marking is enabled (see `set_blacken_enabled`).
    pub fn blacken_enabled(&self) -> bool {
        self.blacken_enabled.load(ORD)
    }

    /// The current heap goal in bytes.
    // Go: runtime/mgcpacer.go:990-994 (2c3c1caad2)
    pub fn heap_goal(&self) -> u64 {
        let (goal, _) = self.heap_goal_internal();
        goal
    }

    /// The heap goal, and the minimum trigger, which is always `<= goal`.
    // Go: runtime/mgcpacer.go:996-1043 (2c3c1caad2)
    fn heap_goal_internal(&self) -> (u64, u64) {
        let mut min_trigger = 0;
        // Start with the goal calculated for gcPercent.
        let mut goal = self.gc_percent_heap_goal.load(ORD);

        // Check if the memory-limit-based goal is smaller, and if so, pick that.
        let new_goal = self.memory_limit_heap_goal();
        if new_goal < goal {
            goal = new_goal;
        } else {
            // We're not limited by the memory limit goal, so perform a series of
            // adjustments that might move the goal forward in a variety of circumstances.

            let sweep_dist_trigger = self.sweep_dist_min_trigger.load(ORD);
            if sweep_dist_trigger > goal {
                // Set the goal to maintain a minimum sweep distance since
                // the last call to commit. Note that we never want to do this
                // if we're in the memory limit regime, because it could push
                // the goal up.
                goal = sweep_dist_trigger;
            }
            // Since we ignore the sweep distance trigger in the memory
            // limit regime, we need to ensure we don't propagate it to
            // the trigger, because it could cause a violation of the
            // invariant that the trigger < goal.
            min_trigger = sweep_dist_trigger;

            // Ensure that the heap goal is at least a little larger than
            // the point at which we triggered. This may not be the case if GC
            // start is delayed or if the allocation that pushed gcController.heapLive
            // over trigger is large or if the trigger is really close to
            // GOGC. Assist is proportional to this distance, so enforce a
            // minimum distance, even if it means going over the GOGC goal
            // by a tiny bit.
            //
            // Ignore this if we're in the memory limit regime: we'd prefer to
            // have the GC respond hard about how close we are to the goal than to
            // push the goal back in such a manner that it could cause us to exceed
            // the memory limit.
            const MIN_RUNWAY: u64 = 64 << 10;
            let triggered = self.triggered.load(ORD);
            if triggered != u64::MAX && goal < triggered.wrapping_add(MIN_RUNWAY) {
                goal = triggered.wrapping_add(MIN_RUNWAY);
            }
        }
        (goal, min_trigger)
    }

    /// The heap goal derived from the memory limit.
    // Go: runtime/mgcpacer.go:1045-1158 (2c3c1caad2)
    fn memory_limit_heap_goal(&self) -> u64 {
        // Start by pulling out some values we'll need. Be careful about overflow.
        //
        // Go retries until heapFree+heapAlloc <= mappedReady, which only fails
        // transiently because the stats are updated independently, and
        // deadlocks on a persistent accounting error. We retry a bounded
        // number of times and then clamp (see the deviations document).
        let mut heap_free;
        let mut heap_alloc;
        let mut mapped_ready;
        let mut tries = 0;
        loop {
            heap_free = self.heap_free.load(ORD); // Free and unscavenged memory.
            heap_alloc = self.heap_alloc.load(ORD); // Heap object bytes in use.
            mapped_ready = self.mapped_ready.load(ORD); // Total unreleased mapped memory.
            if heap_free.saturating_add(heap_alloc) <= mapped_ready {
                break;
            }
            tries += 1;
            if tries >= 100 {
                mapped_ready = heap_free.saturating_add(heap_alloc);
                break;
            }
            std::hint::spin_loop();
        }

        // Below we compute a goal from memoryLimit. See Go's comment at
        // runtime/mgcpacer.go:1063-1115 for the derivation:
        //
        //    goal := memoryLimit - ((mappedReady - heapFree - heapAlloc) + max(mappedReady - memoryLimit, 0))
        //                    ^1                                    ^2
        //    goal -= goal / 100 * memoryLimitHeapGoalHeadroomPercent
        //    ^3
        //
        // Term 1 is non-heap overhead, term 2 the amount by which we exceed the
        // limit, and term 3 headroom for pacing inaccuracies (with a fixed
        // minimum for small heaps).

        let memory_limit = self.memory_limit.load(ORD) as u64;

        // Compute term 1.
        let non_heap_memory = mapped_ready - heap_free - heap_alloc;

        // Compute term 2.
        let mut overage = 0;
        if mapped_ready > memory_limit {
            overage = mapped_ready - memory_limit;
        }

        if non_heap_memory.wrapping_add(overage) >= memory_limit {
            // We're at a point where non-heap memory exceeds the memory limit on its own.
            // There's honestly not much we can do here but just trigger GCs continuously
            // and let the CPU limiter reign that in. Something has to give at this point.
            // Set it to heapMarked, the lowest possible goal.
            return self.heap_marked.load(ORD);
        }

        // Compute the goal.
        let mut goal = memory_limit - (non_heap_memory + overage);

        // Apply some headroom to the goal to account for pacing inaccuracies and to reduce
        // the impact of scavenging at allocation time in response to a high allocation rate
        // when GOGC=off. See issue #57069. Also, be careful about small limits.
        let mut headroom = goal / 100 * MEMORY_LIMIT_HEAP_GOAL_HEADROOM_PERCENT;
        if headroom < MEMORY_LIMIT_MIN_HEAP_GOAL_HEADROOM {
            // Set a fixed minimum to deal with the particularly large effect pacing inaccuracies
            // have for smaller heaps.
            headroom = MEMORY_LIMIT_MIN_HEAP_GOAL_HEADROOM;
        }
        if goal < headroom || goal - headroom < headroom {
            goal = headroom;
        } else {
            goal -= headroom;
        }
        // Don't let us go below the live heap. A heap goal below the live heap doesn't make sense.
        let heap_marked = self.heap_marked.load(ORD);
        if goal < heap_marked {
            goal = heap_marked;
        }
        goal
    }

    /// The current trigger and heap goal, as `(trigger, heap_goal)`, in bytes.
    ///
    /// A cycle should start when `heap_live() >= trigger`. The trigger is
    /// always `<= heap_goal`.
    // Go: runtime/mgcpacer.go:1181-1255 (2c3c1caad2)
    pub fn trigger(&self) -> (u64, u64) {
        let (goal, mut min_trigger) = self.heap_goal_internal();
        let heap_marked = self.heap_marked.load(ORD);

        // Invariant: the trigger must always be less than the heap goal.
        //
        // Note that the memory limit sets a hard maximum on our heap goal,
        // but the live heap may grow beyond it.

        if heap_marked >= goal {
            // The goal should never be smaller than heapMarked, but let's be
            // defensive about it. The only reasonable trigger here is one that
            // causes a continuous GC cycle at heapMarked, but respect the goal
            // if it came out as smaller than that.
            return (goal, goal);
        }

        // Below this point, c.heapMarked < goal.

        // heapMarked is our absolute minimum, and it's possible the trigger
        // bound we get from heapGoalinternal is less than that.
        if min_trigger < heap_marked {
            min_trigger = heap_marked;
        }

        // If we let the trigger go too low, then if the application
        // is allocating very rapidly we might end up in a situation
        // where we're allocating black during a nearly always-on GC.
        // The result of this is a growing heap and ultimately an
        // increase in RSS. By capping us at a point >0, we're essentially
        // saying that we're OK using more CPU during the GC to prevent
        // this growth in RSS.
        let trigger_lower_bound =
            ((goal - heap_marked) / TRIGGER_RATIO_DEN) * MIN_TRIGGER_RATIO_NUM + heap_marked;
        if min_trigger < trigger_lower_bound {
            min_trigger = trigger_lower_bound;
        }

        // For small heaps, set the max trigger point at maxTriggerRatio of the way
        // from the live heap to the heap goal. This ensures we always have *some*
        // headroom when the GC actually starts. For larger heaps, set the max trigger
        // point at the goal, minus the minimum heap size.
        //
        // This choice follows from the fact that the minimum heap size is chosen
        // to reflect the costs of a GC with no work to do. With a large heap but
        // very little scan work to perform, this gives us exactly as much runway
        // as we would need, in the worst case.
        let mut max_trigger =
            ((goal - heap_marked) / TRIGGER_RATIO_DEN) * MAX_TRIGGER_RATIO_NUM + heap_marked;
        if goal > DEFAULT_HEAP_MINIMUM && goal - DEFAULT_HEAP_MINIMUM > max_trigger {
            max_trigger = goal - DEFAULT_HEAP_MINIMUM;
        }
        max_trigger = max_trigger.max(min_trigger);

        // Compute the trigger from our bounds and the runway stored by commit.
        let runway = self.runway.load(ORD);
        let mut trigger = if runway > goal {
            min_trigger
        } else {
            goal - runway
        };
        trigger = trigger.max(min_trigger);
        trigger = trigger.min(max_trigger);
        assert!(
            trigger <= goal,
            "produced a trigger greater than the heap goal: trigger={} heapGoal={} minTrigger={} maxTrigger={}",
            trigger,
            goal,
            min_trigger,
            max_trigger
        );
        (trigger, goal)
    }

    /// Recompute the pacing parameters that derive the trigger and the heap
    /// goal: the `gc_percent` goal and the runway.
    ///
    /// * `is_sweep_done`: whether sweeping of the previous cycle is complete.
    ///   If not, the trigger is kept at least `SWEEP_MIN_HEAP_DISTANCE` above
    ///   the current `heap_live`, to give the sweeper room.
    ///
    /// Depends on `gc_percent`, `heap_marked` and `heap_live`, which must be up
    /// to date. If marking is running, call `revise` afterwards (or use
    /// `controller_commit`). Call with the world stopped or with exclusive
    /// access to the pacer's configuration.
    // Go: runtime/mgcpacer.go:1257-1328 (2c3c1caad2)
    pub fn commit(&self, is_sweep_done: bool) {
        if is_sweep_done {
            // The sweep is done, so there aren't any restrictions on the trigger
            // we need to think about.
            self.sweep_dist_min_trigger.store(0, ORD);
        } else {
            // Concurrent sweep happens in the heap growth
            // from gcController.heapLive to trigger. Make sure we
            // give the sweeper some runway if it doesn't have enough.
            self.sweep_dist_min_trigger.store(
                self.heap_live
                    .load(ORD)
                    .wrapping_add(SWEEP_MIN_HEAP_DISTANCE),
                ORD,
            );
        }

        let heap_marked = self.heap_marked.load(ORD);
        let last_stack_scan = self.last_stack_scan.load(ORD);
        let globals_scan = self.globals_scan.load(ORD);

        // Compute the next GC goal, which is when the allocated heap
        // has grown by GOGC/100 over where it started the last cycle,
        // plus additional runway for non-heap sources of GC work.
        let mut gc_percent_heap_goal = u64::MAX;
        let gc_percent = self.gc_percent.load(ORD);
        if gc_percent >= 0 {
            gc_percent_heap_goal = heap_marked.wrapping_add(
                heap_marked
                    .wrapping_add(last_stack_scan)
                    .wrapping_add(globals_scan)
                    .wrapping_mul(gc_percent as u64)
                    / 100,
            );
        }
        // Apply the minimum heap size here. It's defined in terms of gcPercent
        // and is only updated by functions that call commit.
        let heap_minimum = self.heap_minimum.load(ORD);
        if gc_percent_heap_goal < heap_minimum {
            gc_percent_heap_goal = heap_minimum;
        }
        self.gc_percent_heap_goal.store(gc_percent_heap_goal, ORD);

        // Compute the amount of runway we want the GC to have by using our
        // estimate of the cons/mark ratio.
        //
        // The idea is to take our expected scan work, and multiply it by
        // the cons/mark ratio to determine how long it'll take to complete
        // that scan work in terms of bytes allocated. This gives us our GC's
        // runway.
        //
        // However, the cons/mark ratio is a ratio of rates per CPU-second, but
        // here we care about the relative rates for some division of CPU
        // resources among the mutator and the GC.
        //
        // To summarize, we have B / cpu-ns, and we want B / ns. We get that
        // by multiplying by our desired division of CPU resources. We choose
        // to express CPU resources as GOMAPROCS*fraction. Note that because
        // we're working with a ratio here, we can omit the number of CPU cores,
        // because they'll appear in the numerator and denominator and cancel out.
        // As a result, this is basically just "weighing" the cons/mark ratio by
        // our desired division of resources.
        //
        // Furthermore, by setting the runway so that CPU resources are divided
        // this way, assuming that the cons/mark ratio is correct, we make that
        // division a reality.
        let scan_expected = self
            .last_heap_scan
            .load(ORD)
            .wrapping_add(last_stack_scan)
            .wrapping_add(globals_scan);
        self.runway.store(
            ((self.cons_mark.load() * (1.0 - GC_GOAL_UTILIZATION) / GC_GOAL_UTILIZATION)
                * scan_expected as f64) as u64,
            ORD,
        );
    }

    /// `commit`, followed by `revise` if marking is running, returning the new
    /// `(trigger, heap_goal)`. This is what Go does at the end of every cycle
    /// and whenever `gc_percent` or the memory limit changes.
    ///
    /// * `is_sweep_done`: see `commit`.
    /// * `gc_off`: whether the GC is between cycles (Go: `gcphase == _GCoff`).
    ///
    /// Go also re-paces the sweeper and the scavenger here. Those are the
    /// caller's job.
    // Go: runtime/mgcpacer.go:1507-1537 (2c3c1caad2)
    pub fn controller_commit(&self, is_sweep_done: bool, gc_off: bool) -> (u64, u64) {
        self.commit(is_sweep_done);

        // Update mark pacing.
        if !gc_off {
            self.revise();
        }

        self.trigger()
    }

    /// Set `gc_percent` (see `new`). Returns the old value. Call
    /// `controller_commit` afterwards.
    // Go: runtime/mgcpacer.go:1330-1347 (2c3c1caad2)
    pub fn set_gc_percent(&self, gc_percent: Option<u32>) -> Option<u32> {
        let out = self.set_gc_percent_raw(Self::gc_percent_to_raw(gc_percent));
        u32::try_from(out).ok()
    }

    // Go: runtime/mgcpacer.go:1330-1347 (2c3c1caad2)
    fn set_gc_percent_raw(&self, mut gc_percent: i32) -> i32 {
        let out = self.gc_percent.load(ORD);
        if gc_percent < 0 {
            gc_percent = -1;
        }
        // N.B. As in Go, a negative gc_percent wraps around here, giving a huge
        // heap_minimum, which is harmless because the gc_percent goal is then
        // u64::MAX anyway.
        self.heap_minimum.store(
            DEFAULT_HEAP_MINIMUM.wrapping_mul(gc_percent as u64) / 100,
            ORD,
        );
        self.gc_percent.store(gc_percent, ORD);
        out
    }

    /// Set the soft memory limit (see `new`). Returns the old value (`None`
    /// for no limit). Call `controller_commit` afterwards.
    // Go: runtime/mgcpacer.go:1379-1394 (2c3c1caad2)
    pub fn set_memory_limit(&self, mem_limit: Option<u64>) -> Option<u64> {
        let out = self.set_memory_limit_raw(Self::memory_limit_to_raw(mem_limit));
        if out == i64::MAX {
            None
        } else {
            Some(out as u64)
        }
    }

    // Go: runtime/mgcpacer.go:1379-1394 (2c3c1caad2)
    fn set_memory_limit_raw(&self, memory_limit: i64) -> i64 {
        let out = self.memory_limit.load(ORD);
        if memory_limit >= 0 {
            self.memory_limit.store(memory_limit, ORD);
        }
        out
    }

    /// Update the memory statistics used by the memory-limit goal.
    ///
    /// * `mapped_ready`: all memory counted against the memory limit (Go: total
    ///   mapped memory not released to the OS, for heap and non-heap uses).
    /// * `heap_free`: heap memory that is free (not in any span) but not
    ///   released to the OS.
    /// * `heap_alloc`: heap memory occupied by objects (Go:
    ///   `totalAlloc - totalFree`).
    ///
    /// `heap_free + heap_alloc <= mapped_ready` should hold. Everything else
    /// in `mapped_ready` is treated as non-heap overhead.
    pub fn set_memory_stats(&self, mapped_ready: u64, heap_free: u64, heap_alloc: u64) {
        self.heap_free.store(heap_free, ORD);
        self.heap_alloc.store(heap_alloc, ORD);
        self.mapped_ready.store(mapped_ready, ORD);
    }

    /// Scan work that a mutator assist must do per byte allocated.
    pub fn assist_work_per_byte(&self) -> f64 {
        self.assist_work_per_byte.load()
    }

    /// Bytes a mutator may allocate per unit of scan work (`1 /
    /// assist_work_per_byte`, updated separately).
    pub fn assist_bytes_per_work(&self) -> f64 {
        self.assist_bytes_per_work.load()
    }

    /// The live heap estimate in bytes (Go's `heapLive`).
    pub fn heap_live(&self) -> u64 {
        self.heap_live.load(ORD)
    }

    /// The scannable heap in bytes (Go's `heapScan`).
    pub fn heap_scan(&self) -> u64 {
        self.heap_scan.load(ORD)
    }

    /// The bytes marked by the previous cycle (Go's `heapMarked`).
    pub fn heap_marked(&self) -> u64 {
        self.heap_marked.load(ORD)
    }

    /// The `heap_live` at which the current cycle started, or `u64::MAX`
    /// outside marking.
    pub fn triggered(&self) -> u64 {
        self.triggered.load(ORD)
    }

    /// The heap goal at the end of the last cycle.
    pub fn last_heap_goal(&self) -> u64 {
        self.last_heap_goal.load(ORD)
    }

    /// The configured `gc_percent` (`None` if off).
    pub fn gc_percent(&self) -> Option<u32> {
        u32::try_from(self.gc_percent.load(ORD)).ok()
    }

    /// The configured soft memory limit (`None` if none).
    pub fn memory_limit(&self) -> Option<u64> {
        let l = self.memory_limit.load(ORD);
        if l == i64::MAX {
            None
        } else {
            Some(l as u64)
        }
    }

    /// The current cons/mark estimate.
    pub fn cons_mark(&self) -> f64 {
        self.cons_mark.load()
    }

    /// The runway (bytes) computed by the last `commit`.
    pub fn runway(&self) -> u64 {
        self.runway.load(ORD)
    }

    /// The number of dedicated mark workers still to be started this cycle.
    /// A worker that takes a slot decrements it.
    pub fn dedicated_mark_workers_needed(&self) -> &AtomicI64 {
        &self.dedicated_mark_workers_needed
    }

    /// The fractional utilisation goal for this cycle (0 if none).
    pub fn fractional_utilization_goal(&self) -> f64 {
        self.fractional_utilization_goal.load()
    }

    /// The background scan credit (scan work), added to by workers and stolen
    /// by assists. Reset by `start_cycle`.
    pub fn bg_scan_credit(&self) -> &AtomicI64 {
        &self.bg_scan_credit
    }

    /// Nanoseconds of assist time reported this cycle.
    pub fn assist_time(&self) -> i64 {
        self.assist_time.load(ORD)
    }

    /// Nanoseconds spent in dedicated, fractional and idle workers this cycle.
    pub fn mark_time(&self) -> (i64, i64, i64) {
        (
            self.dedicated_mark_time.load(ORD),
            self.fractional_mark_time.load(ORD),
            self.idle_mark_time.load(ORD),
        )
    }
}
