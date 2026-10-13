// This file is derived from the Go runtime's pacer tests,
// `runtime/mgcpacer_test.go` and the test hooks in `runtime/export_test.go`,
// at Go commit 2c3c1caad2. It is a port of Go's code, not a product of the Go
// project, and implies no endorsement by Google or the Go contributors. The
// original copyright notice and licence follow.
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

//! Ports of Go's `TestGcPacer` scenarios and its `gcExecTest` simulation.
//!
//! The simulation drives the pacer through idealised GC cycles: allocation and
//! scanning proceed at given rates, assists keep the ratio of the two at the
//! pacer's assist ratio, and the resulting GC CPU utilisation and heap peak are
//! checked against the same bounds as Go's test.

use super::*;

const MILLISECOND: i64 = 1_000_000;
const MICROSECOND: i64 = 1_000;

/// Test hooks, mirroring Go's `GCController` wrappers.
// Go: runtime/export_test.go:1460-1529 (2c3c1caad2)
impl FeedbackPacer {
    fn test_start_cycle(
        &self,
        stack_size: u64,
        globals_size: u64,
        scannable_frac: f64,
        gomaxprocs: usize,
    ) {
        let (mut trigger, _) = self.trigger();
        let heap_marked = self.heap_marked.load(ORD);
        if heap_marked > trigger {
            trigger = heap_marked;
        }
        self.max_stack_scan.store(stack_size, ORD);
        self.globals_scan.store(globals_size, ORD);
        self.heap_live.store(trigger, ORD);
        self.heap_scan.fetch_add(
            ((trigger - heap_marked) as f64 * scannable_frac) as i64 as u64,
            ORD,
        );
        self.start_cycle(0, gomaxprocs, false);
    }

    fn test_revise(&self, d: ReviseDelta) {
        self.heap_live.fetch_add(d.heap_live as u64, ORD);
        self.heap_scan.fetch_add(d.heap_scan as u64, ORD);
        self.heap_scan_work.fetch_add(d.heap_scan_work, ORD);
        self.stack_scan_work.fetch_add(d.stack_scan_work, ORD);
        self.globals_scan_work.fetch_add(d.globals_scan_work, ORD);
        self.revise();
    }

    fn test_end_cycle(&self, bytes_marked: u64, assist_time: i64, elapsed: i64, gomaxprocs: usize) {
        self.assist_time.store(assist_time, ORD);
        self.end_cycle(elapsed, gomaxprocs);
        self.reset_live(bytes_marked);
        self.commit(false);
    }
}

// Go: runtime/export_test.go:1507-1513 (2c3c1caad2)
struct ReviseDelta {
    heap_live: i64,
    heap_scan: i64,
    heap_scan_work: i64,
    stack_scan_work: i64,
    globals_scan_work: i64,
}

// ---------------------------------------------------------------------------
// float64Stream
// Go: runtime/mgcpacer_test.go:875-1019 (2c3c1caad2)
// ---------------------------------------------------------------------------

/// An infinite stream of `f64` values (Go's `float64Stream`).
struct Stream(Box<dyn FnMut() -> f64>);

impl Stream {
    fn next(&mut self) -> f64 {
        (self.0)()
    }

    /// Delay: zero for `cycles` steps, then `self`.
    fn delay(mut self, cycles: usize) -> Stream {
        let mut zeroes = 0;
        Stream(Box::new(move || {
            if zeroes < cycles {
                zeroes += 1;
                return 0.0;
            }
            self.next()
        }))
    }

    fn offset(mut self, amt: f64) -> Stream {
        Stream(Box::new(move || self.next() + amt))
    }

    fn sum(mut self, mut fs: Vec<Stream>) -> Stream {
        Stream(Box::new(move || {
            let mut sum = self.next();
            for s in fs.iter_mut() {
                sum += s.next();
            }
            sum
        }))
    }

    fn quantize(mut self, mult: f64) -> Stream {
        Stream(Box::new(move || {
            let r = self.next() / mult;
            if r < 0.0 {
                return r.ceil() * mult;
            }
            r.floor() * mult
        }))
    }

    fn min(mut self, min: f64) -> Stream {
        Stream(Box::new(move || f64::max(min, self.next())))
    }

    fn limit(mut self, min: f64, max: f64) -> Stream {
        Stream(Box::new(move || {
            let mut v = self.next();
            if v < min {
                v = min;
            } else if v > max {
                v = max;
            }
            v
        }))
    }
}

fn constant(c: f64) -> Stream {
    Stream(Box::new(move || c))
}

/// A single peak of amplitude `amp`, then zeroes (Kronecker delta).
fn unit(amp: f64) -> Stream {
    let mut dropped = false;
    Stream(Box::new(move || {
        if dropped {
            return 0.0;
        }
        dropped = true;
        amp
    }))
}

fn oscillate(amp: f64, phase: f64, period: usize) -> Stream {
    let mut cycle = 0usize;
    Stream(Box::new(move || {
        let p = cycle as f64 / period as f64 * 2.0 * std::f64::consts::PI + phase;
        cycle += 1;
        if cycle == period {
            cycle = 0;
        }
        p.sin() * amp
    }))
}

/// From zero to `height` over `length` steps.
fn ramp(height: f64, length: usize) -> Stream {
    let mut cycle = 0usize;
    Stream(Box::new(move || {
        let h = height * cycle as f64 / length as f64;
        if cycle < length {
            cycle += 1;
        }
        h
    }))
}

/// Uniform random numbers in `[-amp, amp)`.
///
/// Go uses `math/rand` with the given seed. That generator (an additive lagged
/// Fibonacci generator seeded from a 607-entry table) is not reproduced here;
/// SplitMix64 is used instead, so the jitter differs from Go's while having
/// the same distribution. See the deviations document.
fn random(amp: f64, seed: u64) -> Stream {
    let mut state = seed;
    Stream(Box::new(move || {
        // SplitMix64.
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        let f = (z >> 11) as f64 / (1u64 << 53) as f64;
        ((f - 0.5) * 2.0) * amp
    }))
}

// ---------------------------------------------------------------------------
// gcExecTest
// Go: runtime/mgcpacer_test.go:759-873 (2c3c1caad2)
// ---------------------------------------------------------------------------

type Checker = fn(&mut Errors, &[GcCycleResult]);

struct GcExecTest {
    name: &'static str,

    gc_percent: i32,
    memory_limit: i64,
    globals_bytes: u64,
    n_cores: usize,

    alloc_rate: Stream,     // > 0, KiB / cpu-ms
    scan_rate: Stream,      // > 0, KiB / cpu-ms
    growth_rate: Stream,    // > 0
    scannable_frac: Stream, // Clamped to [0, 1]
    stack_bytes: Stream,    // Multiple of 2048.
    length: usize,

    checker: Checker,
}

/// An arbitrary minimum for allocRate, scanRate, and growthRate.
const MIN_RATE: f64 = 0.0001;

struct GcCycle {
    alloc_rate: f64,
    scan_rate: f64,
    growth_rate: f64,
    scannable_frac: f64,
    stack_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
struct GcCycleResult {
    cycle: usize,

    // These come directly from the pacer, so u64.
    heap_live: u64,
    heap_trigger: u64,
    heap_goal: u64,
    heap_peak: u64,

    // These are produced by the simulation, so i64 and f64 are more
    // appropriate, so that we can check for bad states in the simulation.
    heap_scannable: i64,
    gc_utilization: f64,
}

impl GcCycleResult {
    fn goal_ratio(&self) -> f64 {
        self.heap_peak as f64 / self.heap_goal as f64
    }

    fn runway(&self) -> f64 {
        (self.heap_goal - self.heap_trigger) as f64
    }

    fn trigger_ratio(&self) -> f64 {
        (self.heap_trigger - self.heap_live) as f64 / (self.heap_goal - self.heap_live) as f64
    }
}

impl std::fmt::Display for GcCycleResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:2.1}% {}->{}->{} (goal: {})",
            self.cycle,
            self.gc_utilization * 100.0,
            self.heap_live,
            self.heap_trigger,
            self.heap_peak,
            self.heap_goal
        )
    }
}

/// Collected failures, like Go's `t.Errorf`.
#[derive(Default)]
struct Errors(Vec<String>);

impl Errors {
    fn error(&mut self, msg: String) {
        self.0.push(msg);
    }

    fn assert_in_epsilon(&mut self, name: &str, a: f64, b: f64, epsilon: f64) {
        self.assert_in_range(name, a, b - epsilon, b + epsilon);
    }

    fn assert_in_range(&mut self, name: &str, a: f64, min: f64, max: f64) {
        if a < min || a > max {
            self.error(format!(
                "{} not in range ({:.6}, {:.6}): {:.6}",
                name, min, max, a
            ));
        }
    }
}

impl GcExecTest {
    // Go: runtime/mgcpacer_test.go:781-789 (2c3c1caad2)
    fn next(&mut self) -> GcCycle {
        GcCycle {
            alloc_rate: self.alloc_rate.next().max(MIN_RATE),
            scan_rate: self.scan_rate.next().max(MIN_RATE),
            growth_rate: self.growth_rate.next().max(MIN_RATE),
            scannable_frac: self.scannable_frac.next().clamp(0.0, 1.0),
            stack_bytes: {
                let r = self.stack_bytes.next() / 2048.0;
                let q = if r < 0.0 { r.ceil() } else { r.floor() } * 2048.0;
                f64::max(0.0, q) as u64
            },
        }
    }

    // Go: runtime/mgcpacer_test.go:791-821 (2c3c1caad2)
    fn check(&self, errors: &mut Errors, results: &[GcCycleResult]) -> Result<(), String> {
        let n = results.len();
        match n {
            0 => return Err("no results passed to check".into()),
            1 => {
                if results[0].cycle != 1 {
                    errors.error("first cycle has incorrect number".into());
                }
            }
            _ => {
                if results[n - 1].cycle != results[n - 2].cycle + 1 {
                    errors.error("cycle numbers out of order".into());
                }
            }
        }
        let u = results[n - 1].gc_utilization;
        if !(0.0..=1.0).contains(&u) {
            return Err("GC utilization not within acceptable bounds".into());
        }
        if results[n - 1].heap_scannable < 0 {
            return Err("heapScannable is negative".into());
        }

        // Run the test-specific checker.
        (self.checker)(errors, results);
        Ok(())
    }

    /// Run the simulation and return the per-cycle results, or the failures.
    // Go: runtime/mgcpacer_test.go:606-755 (2c3c1caad2)
    fn run(mut self) -> Result<Vec<GcCycleResult>, String> {
        const INITIAL_HEAP_BYTES: i64 = 256 << 10;
        let mut log = String::new();
        let mut errors = Errors::default();

        let c = FeedbackPacer::new_raw(self.gc_percent, self.memory_limit);
        let mut bytes_allocated_black_last: i64 = 0;
        let mut results: Vec<GcCycleResult> = Vec::with_capacity(self.length);
        for i in 0..self.length {
            let cycle = self.next();
            c.test_start_cycle(
                cycle.stack_bytes,
                self.globals_bytes,
                cycle.scannable_frac,
                self.n_cores,
            );

            // Update pacer incrementally as we complete scan work.
            const REVISE_PERIOD: i64 = 500 * MICROSECOND;
            const RATE_CONV: f64 = 1024.0 * REVISE_PERIOD as f64 / MILLISECOND as f64;
            let next_heap_marked: i64 = if i == 0 {
                INITIAL_HEAP_BYTES
            } else {
                ((c.heap_marked() as i64 - bytes_allocated_black_last) as f64 * cycle.growth_rate)
                    as i64
            };
            let mut work_left: [i64; 3] = [
                self.globals_bytes as i64,
                cycle.stack_bytes as i64,
                (next_heap_marked as f64 * cycle.scannable_frac) as i64,
            ];
            // Do globals work first, then stacks, then heap.
            let do_work = |work_left: &mut [i64; 3], mut work: i64| -> [i64; 3] {
                let mut deltas = [0i64; 3];
                for (i, left) in work_left.iter_mut().enumerate() {
                    if *left == 0 {
                        continue;
                    }
                    if *left > work {
                        deltas[i] += work;
                        *left -= work;
                        break;
                    } else {
                        deltas[i] += *left;
                        work -= *left;
                        *left = 0;
                    }
                }
                deltas
            };
            let mut gc_duration: i64 = 0;
            let mut assist_time: i64 = 0;
            let mut bytes_allocated_black: i64 = 0;
            while work_left.iter().sum::<i64>() > 0 {
                // Simulate GC assist pacing. See Go's comment at
                // runtime/mgcpacer_test.go:657-694: with the assist ratio
                // fixed over a revise period, the GC CPU utilisation is
                //
                //     u = (assistRatio * allocRate) / (assistRatio * allocRate + scanRate)
                //
                // rounded up to the background utilisation, since dedicated
                // workers always run.
                let assist_ratio = c.assist_work_per_byte();
                let mut utilization = assist_ratio * cycle.alloc_rate
                    / (assist_ratio * cycle.alloc_rate + cycle.scan_rate);
                if utilization < GC_BACKGROUND_UTILIZATION {
                    utilization = GC_BACKGROUND_UTILIZATION;
                }

                // Knowing the utilization, calculate bytesScanned and bytesAllocated.
                let bytes_scanned =
                    (cycle.scan_rate * RATE_CONV * self.n_cores as f64 * utilization) as i64;
                let bytes_allocated =
                    (cycle.alloc_rate * RATE_CONV * self.n_cores as f64 * (1.0 - utilization))
                        as i64;

                // Subtract work from our model.
                let [globals_scanned, stack_scanned, heap_scanned] =
                    do_work(&mut work_left, bytes_scanned);

                // doWork may not use all of bytesScanned.
                // In this case, the GC actually ends sometime in this period.
                // Let's figure out when, exactly, and adjust bytesAllocated too.
                let mut actual_elapsed = REVISE_PERIOD;
                let mut actual_allocated = bytes_allocated;
                let actual_scanned = globals_scanned + stack_scanned + heap_scanned;
                if actual_scanned < bytes_scanned {
                    actual_elapsed = (actual_scanned as f64 * REVISE_PERIOD as f64
                        / (cycle.scan_rate * RATE_CONV * self.n_cores as f64 * utilization))
                        as i64;
                    actual_allocated = (cycle.alloc_rate * RATE_CONV * actual_elapsed as f64
                        / REVISE_PERIOD as f64
                        * self.n_cores as f64
                        * (1.0 - utilization)) as i64;
                }

                // Ask the pacer to revise.
                c.test_revise(ReviseDelta {
                    heap_live: actual_allocated,
                    heap_scan: (actual_allocated as f64 * cycle.scannable_frac) as i64,
                    heap_scan_work: heap_scanned,
                    stack_scan_work: stack_scanned,
                    globals_scan_work: globals_scanned,
                });

                // Accumulate variables.
                assist_time += (actual_elapsed as f64
                    * self.n_cores as f64
                    * (utilization - GC_BACKGROUND_UTILIZATION))
                    as i64;
                gc_duration += actual_elapsed;
                bytes_allocated_black += actual_allocated;
            }

            // Put together the results, log them, and concatenate them.
            let result = GcCycleResult {
                cycle: i + 1,
                heap_live: c.heap_marked(),
                heap_scannable: ((c.heap_marked() as i64 - bytes_allocated_black_last) as f64
                    * cycle.scannable_frac) as i64,
                heap_trigger: c.triggered(),
                heap_peak: c.heap_live(),
                heap_goal: c.heap_goal(),
                gc_utilization: assist_time as f64 / (gc_duration as f64 * self.n_cores as f64)
                    + GC_BACKGROUND_UTILIZATION,
            };
            log.push_str(&format!("GC {}\n", result));
            results.push(result);

            // Run the checker for this test.
            let before = errors.0.len();
            if let Err(fatal) = self.check(&mut errors, &results) {
                errors.error(format!("cycle {}: fatal: {}", i + 1, fatal));
                break;
            }
            for e in errors.0[before..].iter_mut() {
                *e = format!("cycle {}: {}", i + 1, e);
            }

            c.test_end_cycle(
                (next_heap_marked + bytes_allocated_black) as u64,
                assist_time,
                gc_duration,
                self.n_cores,
            );

            bytes_allocated_black_last = bytes_allocated_black;
        }

        if errors.0.is_empty() {
            Ok(results)
        } else {
            Err(format!(
                "{} failed:\n{}\nlog:\n{}",
                self.name,
                errors.0.join("\n"),
                log
            ))
        }
    }
}

/// Go: `applyMemoryLimitHeapGoalHeadroom`.
// Go: runtime/mgcpacer_test.go:1021-1032 (2c3c1caad2)
fn apply_memory_limit_heap_goal_headroom(mut goal: u64) -> u64 {
    let mut headroom = goal / 100 * MEMORY_LIMIT_HEAP_GOAL_HEADROOM_PERCENT;
    if headroom < MEMORY_LIMIT_MIN_HEAP_GOAL_HEADROOM {
        headroom = MEMORY_LIMIT_MIN_HEAP_GOAL_HEADROOM;
    }
    if goal < headroom || goal - headroom < headroom {
        goal = headroom;
    } else {
        goal -= headroom;
    }
    goal
}

fn run(test: GcExecTest) {
    let length = test.length;
    match test.run() {
        Ok(results) => {
            assert_eq!(results.len(), length);
            // The simulation must actually reach a steady heap.
            assert!(results[length - 1].heap_live > 256 << 10);
        }
        Err(msg) => panic!("{}", msg),
    }
}

// ---------------------------------------------------------------------------
// TestGcPacer scenarios
// Go: runtime/mgcpacer_test.go:16-605 (2c3c1caad2)
// ---------------------------------------------------------------------------

/// The fields shared by most scenarios: GOGC=100, no memory limit, 32 KiB of
/// globals, 8 cores, scan rate 1024, growth to a steady heap after 12 cycles,
/// all of the heap scannable, 8 KiB of stacks, 50 cycles.
fn base(name: &'static str, checker: Checker) -> GcExecTest {
    GcExecTest {
        name,
        gc_percent: 100,
        memory_limit: i64::MAX,
        globals_bytes: 32 << 10,
        n_cores: 8,
        alloc_rate: constant(33.0),
        scan_rate: constant(1024.0),
        growth_rate: constant(2.0).sum(vec![ramp(-1.0, 12)]),
        scannable_frac: constant(1.0),
        stack_bytes: constant(8192.0),
        length: 50,
        checker,
    }
}

/// The checker shared by `Steady`, `SteadyBigStacks`, `SteadyBigGlobals` and
/// the `MaintainMemoryLimit` scenarios (after any scenario-specific checks).
fn check_steady(e: &mut Errors, c: &[GcCycleResult]) {
    let n = c.len();
    if n >= 25 {
        // At this alloc/scan rate, the pacer should be extremely close to the goal utilization.
        e.assert_in_epsilon(
            "GC utilization",
            c[n - 1].gc_utilization,
            GC_GOAL_UTILIZATION,
            0.005,
        );

        // Make sure the pacer settles into a non-degenerate state in at least 25 GC cycles.
        e.assert_in_epsilon(
            "GC utilization",
            c[n - 1].gc_utilization,
            c[n - 2].gc_utilization,
            0.005,
        );
        e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.95, 1.05);
    }
}

/// The checker shared by the step scenarios.
fn check_step(e: &mut Errors, c: &[GcCycleResult]) {
    let n = c.len();
    if (25..50).contains(&n) || n >= 75 {
        // Make sure the pacer settles into a non-degenerate state in at least 25 GC cycles
        // and then is able to settle again after a significant jump in allocation rate.
        e.assert_in_epsilon(
            "GC utilization",
            c[n - 1].gc_utilization,
            c[n - 2].gc_utilization,
            0.005,
        );
        e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.95, 1.05);
    }
}

/// The utilisation checks shared by the slow-allocation scenarios.
fn check_slow_alloc_utilization(e: &mut Errors, c: &[GcCycleResult]) {
    let n = c.len();
    if n > 25 {
        // At this alloc/scan rate, the pacer should be extremely close to the goal utilization.
        e.assert_in_epsilon(
            "GC utilization",
            c[n - 1].gc_utilization,
            GC_GOAL_UTILIZATION,
            0.005,
        );
        // Make sure GC utilization has mostly levelled off.
        e.assert_in_epsilon(
            "GC utilization",
            c[n - 1].gc_utilization,
            c[n - 2].gc_utilization,
            0.05,
        );
        e.assert_in_epsilon(
            "GC utilization",
            c[n - 1].gc_utilization,
            c[11].gc_utilization,
            0.05,
        );
    }
}

/// The checks shared by the scenarios that exceed the memory limit.
fn check_exceed_memory_limit(e: &mut Errors, c: &[GcCycleResult]) {
    let n = c.len();
    if n > 12 {
        // We're way over the memory limit, so we want to make sure our goal is set
        // as low as it possibly can be.
        let (goal, live) = (c[n - 1].heap_goal, c[n - 1].heap_live);
        if goal != live {
            e.error(format!(
                "heap goal is not equal to live heap: {} != {}",
                goal, live
            ));
        }
    }
    if n >= 25 {
        // Due to memory pressure, we should scale to 100% GC CPU utilization.
        // Note that in practice this won't actually happen because of the CPU limiter,
        // but it's not the pacer's job to limit CPU usage.
        e.assert_in_epsilon("GC utilization", c[n - 1].gc_utilization, 1.0, 0.005);

        // Make sure the pacer settles into a non-degenerate state in at least 25 GC cycles.
        // In this case, that just means it's not wavering around a whole bunch.
        e.assert_in_epsilon(
            "GC utilization",
            c[n - 1].gc_utilization,
            c[n - 2].gc_utilization,
            0.005,
        );
    }
}

#[test]
fn gc_pacer_steady() {
    // The most basic test case: a steady-state heap.
    // Growth to an O(MiB) heap, then constant heap size, alloc/scan rates.
    run(base("Steady", check_steady));
}

#[test]
fn gc_pacer_steady_big_stacks() {
    // Same as the steady-state case, but lots of stacks to scan relative to the heap size.
    run(GcExecTest {
        alloc_rate: constant(132.0),
        stack_bytes: constant(2048.0).sum(vec![ramp((128 << 20) as f64, 8)]),
        ..base("SteadyBigStacks", check_steady)
    });
}

#[test]
fn gc_pacer_steady_big_globals() {
    // Same as the steady-state case, but lots of globals to scan relative to the heap size.
    run(GcExecTest {
        globals_bytes: 128 << 20,
        alloc_rate: constant(132.0),
        ..base("SteadyBigGlobals", check_steady)
    });
}

#[test]
fn gc_pacer_step_alloc() {
    // This tests the GC pacer's response to a small change in allocation rate.
    run(GcExecTest {
        alloc_rate: constant(33.0).sum(vec![ramp(66.0, 1).delay(50)]),
        length: 100,
        ..base("StepAlloc", check_step)
    });
}

#[test]
fn gc_pacer_heavy_step_alloc() {
    // This tests the GC pacer's response to a large change in allocation rate.
    run(GcExecTest {
        alloc_rate: constant(33.0).sum(vec![ramp(330.0, 1).delay(50)]),
        length: 100,
        ..base("HeavyStepAlloc", check_step)
    });
}

#[test]
fn gc_pacer_step_scannable_frac() {
    // This tests the GC pacer's response to a change in the fraction of the scannable heap.
    run(GcExecTest {
        alloc_rate: constant(128.0),
        scannable_frac: constant(0.2).sum(vec![unit(0.5).delay(50)]),
        length: 100,
        ..base("StepScannableFrac", check_step)
    });
}

#[test]
fn gc_pacer_high_gc_percent() {
    // Tests the pacer for a high GOGC value with a large heap growth happening
    // in the middle. The purpose of the large heap growth is to check if GC
    // utilization ends up sensitive
    run(GcExecTest {
        gc_percent: 1500,
        alloc_rate: random(7.0, 0x53).offset(165.0),
        growth_rate: constant(2.0).sum(vec![
            ramp(-1.0, 12),
            random(0.01, 0x1),
            unit(14.0).delay(25),
        ]),
        ..base("HighGOGC", |e, c| {
            let n = c.len();
            if n > 12 {
                if n == 26 {
                    // In the 26th cycle there's a heap growth. Overshoot is expected to maintain
                    // a stable utilization, but we should *never* overshoot more than GOGC of
                    // the next cycle.
                    e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.90, 15.0);
                } else {
                    // Give a wider goal range here. With such a high GOGC value we're going to be
                    // forced to undershoot.
                    e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.90, 1.05);
                }

                // Ensure utilization remains stable despite a growth in live heap size
                // at GC #25. This test fails prior to the GC pacer redesign.
                //
                // Because GOGC is so large, we should also be really close to the goal utilization.
                e.assert_in_epsilon(
                    "GC utilization",
                    c[n - 1].gc_utilization,
                    GC_GOAL_UTILIZATION,
                    GC_GOAL_UTILIZATION + 0.03,
                );
                e.assert_in_epsilon(
                    "GC utilization",
                    c[n - 1].gc_utilization,
                    c[n - 2].gc_utilization,
                    0.03,
                );
            }
        })
    });
}

#[test]
fn gc_pacer_osc_alloc() {
    // This test makes sure that in the face of a varying (in this case, oscillating) allocation
    // rate, the pacer does a reasonably good job of staying abreast of the changes.
    run(GcExecTest {
        alloc_rate: oscillate(13.0, 0.0, 8).offset(67.0),
        ..base("OscAlloc", |e, c| {
            let n = c.len();
            if n > 12 {
                // After the 12th GC, the heap will stop growing. Now, just make sure that:
                // 1. Utilization isn't varying _too_ much, and
                // 2. The pacer is mostly keeping up with the goal.
                e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.95, 1.05);
                e.assert_in_range("GC utilization", c[n - 1].gc_utilization, 0.25, 0.3);
            }
        })
    });
}

#[test]
fn gc_pacer_jitter_alloc() {
    // This test is the same as OscAlloc, but instead of oscillating, the allocation rate is jittery.
    run(GcExecTest {
        alloc_rate: random(13.0, 0xf).offset(132.0),
        growth_rate: constant(2.0).sum(vec![ramp(-1.0, 12), random(0.01, 0xe)]),
        ..base("JitterAlloc", |e, c| {
            let n = c.len();
            if n > 12 {
                // After the 12th GC, the heap will stop growing. Now, just make sure that:
                // 1. Utilization isn't varying _too_ much, and
                // 2. The pacer is mostly keeping up with the goal.
                e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.95, 1.025);
                e.assert_in_range("GC utilization", c[n - 1].gc_utilization, 0.25, 0.275);
            }
        })
    });
}

#[test]
fn gc_pacer_heavy_jitter_alloc() {
    // This test is the same as JitterAlloc, but with a much higher allocation rate.
    // The jitter is proportionally the same.
    run(GcExecTest {
        alloc_rate: random(33.0, 0x0).offset(330.0),
        growth_rate: constant(2.0).sum(vec![ramp(-1.0, 12), random(0.01, 0x152)]),
        ..base("HeavyJitterAlloc", |e, c| {
            let n = c.len();
            if n > 13 {
                // After the 12th GC, the heap will stop growing. Now, just make sure that:
                // 1. Utilization isn't varying _too_ much, and
                // 2. The pacer is mostly keeping up with the goal.
                // We start at the 13th here because we want to use the 12th as a reference.
                e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.95, 1.05);
                // Unlike the other tests, GC utilization here will vary more and tend higher.
                // Just make sure it's not going too crazy.
                e.assert_in_epsilon(
                    "GC utilization",
                    c[n - 1].gc_utilization,
                    c[n - 2].gc_utilization,
                    0.05,
                );
                e.assert_in_epsilon(
                    "GC utilization",
                    c[n - 1].gc_utilization,
                    c[11].gc_utilization,
                    0.05,
                );
            }
        })
    });
}

#[test]
fn gc_pacer_small_heap_slow_alloc() {
    // This test sets a slow allocation rate and a small heap (close to the minimum heap size)
    // to try to minimize the difference between the trigger and the goal.
    run(GcExecTest {
        alloc_rate: constant(1.0),
        scan_rate: constant(2048.0),
        growth_rate: constant(2.0).sum(vec![ramp(-1.0, 3)]),
        scannable_frac: constant(0.01),
        ..base("SmallHeapSlowAlloc", |e, c| {
            let n = c.len();
            if n > 4 {
                // After the 4th GC, the heap will stop growing.
                // First, let's make sure we're finishing near the goal, with some extra
                // room because we're probably going to be triggering early.
                e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.925, 1.025);
                // Next, let's make sure there's some minimum distance between the goal
                // and the trigger. It should be proportional to the runway (hence the
                // trigger ratio check, instead of a check against the runway).
                e.assert_in_range("trigger ratio", c[n - 1].trigger_ratio(), 0.925, 0.975);
            }
            check_slow_alloc_utilization(e, c);
        })
    });
}

#[test]
fn gc_pacer_medium_heap_slow_alloc() {
    // This test sets a slow allocation rate and a medium heap (around 10x the min heap size)
    // to try to minimize the difference between the trigger and the goal.
    run(GcExecTest {
        alloc_rate: constant(1.0),
        scan_rate: constant(2048.0),
        growth_rate: constant(2.0).sum(vec![ramp(-1.0, 8)]),
        scannable_frac: constant(0.01),
        ..base("MediumHeapSlowAlloc", |e, c| {
            let n = c.len();
            if n > 9 {
                // After the 4th GC, the heap will stop growing.
                // First, let's make sure we're finishing near the goal, with some extra
                // room because we're probably going to be triggering early.
                e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.925, 1.025);
                // Next, let's make sure there's some minimum distance between the goal
                // and the trigger. It should be proportional to the runway (hence the
                // trigger ratio check, instead of a check against the runway).
                e.assert_in_range("trigger ratio", c[n - 1].trigger_ratio(), 0.925, 0.975);
            }
            check_slow_alloc_utilization(e, c);
        })
    });
}

#[test]
fn gc_pacer_large_heap_slow_alloc() {
    // This test sets a slow allocation rate and a large heap to try to minimize the
    // difference between the trigger and the goal.
    run(GcExecTest {
        alloc_rate: constant(1.0),
        scan_rate: constant(2048.0),
        growth_rate: constant(4.0).sum(vec![ramp(-3.0, 12)]),
        scannable_frac: constant(0.01),
        ..base("LargeHeapSlowAlloc", |e, c| {
            let n = c.len();
            if n > 13 {
                // After the 4th GC, the heap will stop growing.
                // First, let's make sure we're finishing near the goal.
                e.assert_in_range("goal ratio", c[n - 1].goal_ratio(), 0.95, 1.05);
                // Next, let's make sure there's some minimum distance between the goal
                // and the trigger. It should be around the default minimum heap size.
                e.assert_in_range(
                    "runway",
                    c[n - 1].runway(),
                    (DEFAULT_HEAP_MINIMUM - (64 << 10)) as f64,
                    (DEFAULT_HEAP_MINIMUM + (64 << 10)) as f64,
                );
            }
            check_slow_alloc_utilization(e, c);
        })
    });
}

#[test]
fn gc_pacer_steady_memory_limit() {
    // The most basic test case with a memory limit: a steady-state heap.
    // Growth to an O(MiB) heap, then constant heap size, alloc/scan rates.
    // Provide a lot of room for the limit. Essentially, this should behave just like
    // the "Steady" test. Note that we don't simulate non-heap overheads, so the
    // memory limit and the heap limit are identical.
    run(GcExecTest {
        memory_limit: 512 << 20,
        ..base("SteadyMemoryLimit", |e, c| {
            let n = c.len();
            let peak = c[n - 1].heap_peak;
            if peak >= apply_memory_limit_heap_goal_headroom(512 << 20) {
                e.error(format!("peak heap size reaches heap limit: {}", peak));
            }
            check_steady(e, c);
        })
    });
}

#[test]
fn gc_pacer_steady_memory_limit_no_gc_percent() {
    // This is the same as the previous test, but gcPercent = -1, so the heap *should* grow
    // all the way to the peak.
    run(GcExecTest {
        gc_percent: -1,
        memory_limit: 512 << 20,
        ..base("SteadyMemoryLimitNoGCPercent", |e, c| {
            let n = c.len();
            let goal = c[n - 1].heap_goal;
            if goal != apply_memory_limit_heap_goal_headroom(512 << 20) {
                e.error(format!("heap goal is not the heap limit: {}", goal));
            }
            check_steady(e, c);
        })
    });
}

#[test]
fn gc_pacer_exceed_memory_limit() {
    // This test ensures that the pacer doesn't fall over even when the live heap exceeds
    // the memory limit. It also makes sure GC utilization actually rises to push back.
    run(GcExecTest {
        memory_limit: 512 << 20,
        growth_rate: constant(3.5).sum(vec![ramp(-2.5, 12)]),
        ..base("ExceedMemoryLimit", check_exceed_memory_limit)
    });
}

#[test]
fn gc_pacer_exceed_memory_limit_no_gc_percent() {
    // Same as the previous test, but with gcPercent = -1.
    run(GcExecTest {
        gc_percent: -1,
        memory_limit: 512 << 20,
        growth_rate: constant(3.5).sum(vec![ramp(-2.5, 12)]),
        ..base("ExceedMemoryLimitNoGCPercent", |e, c| {
            let n = c.len();
            if n < 10 {
                let goal = c[n - 1].heap_goal;
                if goal != apply_memory_limit_heap_goal_headroom(512 << 20) {
                    e.error(format!("heap goal is not the heap limit: {}", goal));
                }
            }
            check_exceed_memory_limit(e, c);
        })
    });
}

#[test]
fn gc_pacer_maintain_memory_limit() {
    // This test ensures that the pacer maintains the memory limit as the heap grows.
    run(GcExecTest {
        memory_limit: 512 << 20,
        growth_rate: constant(3.0).sum(vec![ramp(-2.0, 12)]),
        ..base("MaintainMemoryLimit", |e, c| {
            let n = c.len();
            if n > 12 {
                // We're trying to saturate the memory limit.
                let goal = c[n - 1].heap_goal;
                if goal != apply_memory_limit_heap_goal_headroom(512 << 20) {
                    e.error(format!("heap goal is not the heap limit: {}", goal));
                }
            }
            // At this alloc/scan rate, the pacer should be extremely close to the goal utilization,
            // even with the additional memory pressure.
            check_steady(e, c);
        })
    });
}

#[test]
fn gc_pacer_maintain_memory_limit_no_gc_percent() {
    // Same as the previous test, but with gcPercent = -1.
    run(GcExecTest {
        gc_percent: -1,
        memory_limit: 512 << 20,
        growth_rate: constant(3.0).sum(vec![ramp(-2.0, 12)]),
        ..base("MaintainMemoryLimitNoGCPercent", |e, c| {
            let n = c.len();
            let goal = c[n - 1].heap_goal;
            if goal != apply_memory_limit_heap_goal_headroom(512 << 20) {
                e.error(format!("heap goal is not the heap limit: {}", goal));
            }
            check_steady(e, c);
        })
    });
}

// ---------------------------------------------------------------------------
// Unit tests of our own for the public API (not ports of Go tests).
// ---------------------------------------------------------------------------

#[test]
fn initial_goal_is_heap_minimum() {
    let c = FeedbackPacer::new(Some(100), None);
    let (trigger, goal) = c.trigger();
    assert_eq!(goal, DEFAULT_HEAP_MINIMUM);
    assert!(trigger <= goal);
    assert_eq!(c.gc_percent(), Some(100));
    assert_eq!(c.memory_limit(), None);

    // GOGC=200 doubles the minimum heap.
    let c = FeedbackPacer::new(Some(200), None);
    assert_eq!(c.heap_goal(), 2 * DEFAULT_HEAP_MINIMUM);
}

#[test]
fn gc_percent_off_uses_memory_limit() {
    let c = FeedbackPacer::new(None, Some(64 << 20));
    assert_eq!(c.gc_percent(), None);
    assert_eq!(
        c.heap_goal(),
        apply_memory_limit_heap_goal_headroom(64 << 20)
    );

    // Non-heap memory reduces the goal.
    c.set_memory_stats(16 << 20, 0, 0);
    assert_eq!(
        c.heap_goal(),
        apply_memory_limit_heap_goal_headroom(48 << 20)
    );

    // Non-heap memory over the limit: the goal falls to heapMarked.
    c.set_memory_stats(80 << 20, 0, 0);
    assert_eq!(c.heap_goal(), c.heap_marked());
}

#[test]
fn gc_percent_goal_and_trigger_after_cycle() {
    let c = FeedbackPacer::new(Some(100), None);
    c.add_globals(1 << 20);
    c.update(16 << 20, 16 << 20);
    c.start_cycle(0, 8, false);
    c.set_blacken_enabled(true);
    assert_eq!(c.triggered(), 16 << 20);
    assert_eq!(c.dedicated_mark_workers_needed().load(ORD), 2);
    assert_eq!(c.fractional_utilization_goal(), 0.0);
    c.add_scan_work(8 << 20, 0, 1 << 20);
    c.update(4 << 20, 0);
    c.set_blacken_enabled(false);
    c.end_cycle(10 * MILLISECOND, 8);
    c.reset_live(8 << 20);
    let (trigger, goal) = c.controller_commit(true, true);
    // goal = heapMarked + (heapMarked + stack + globals) * 100 / 100
    assert_eq!(goal, (8 << 20) + (8 << 20) + (1 << 20));
    assert!(trigger > c.heap_marked() && trigger <= goal);
    assert!(c.cons_mark() > 0.0);
}

#[test]
fn fractional_workers_for_small_procs() {
    // Go: GOMAXPROCS <= 3 or == 6 use fractional workers.
    for (procs, dedicated) in [(1, 0), (2, 0), (3, 0), (4, 1), (6, 1), (8, 2)] {
        let c = FeedbackPacer::new(Some(100), None);
        c.start_cycle(0, procs, false);
        assert_eq!(c.dedicated_mark_workers_needed().load(ORD), dedicated);
        let goal = c.fractional_utilization_goal();
        let total = dedicated as f64 + goal * procs as f64;
        assert!((total - procs as f64 * 0.25).abs() < 1e-9 || goal == 0.0);
    }
}
