// This file is derived from the Go runtime's GC CPU limiter,
// `runtime/mgclimit.go` and its test `runtime/mgclimit_test.go`, at Go commit
// 2c3c1caad2. It is a port of Go's code, not a product of the Go project, and
// implies no endorsement by Google or the Go contributors. The original
// copyright notice and licence follow.
//
// Copyright 2022 The Go Authors. All rights reserved.
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

//! `CpuLimiter`: a port of Go's GC CPU limiter (`gcCPULimiterState` in
//! `runtime/mgclimit.go`).
//!
//! The limiter caps GC CPU utilisation in situations where it might become
//! excessive and inhibit application progress (a death spiral). Its core is a
//! leaky bucket that fills with GC CPU time and drains with mutator CPU time.
//! Because both are unweighted, this sets a conservative limit of 50%; the
//! bucket's capacity (1 CPU-second per CPU) absorbs spikes. While the bucket is
//! full, [`CpuLimiter::limiting`] is true and the GC suppresses assists
//! (design §5.11).
//!
//! The bucket never goes negative, so the GC gets no credit for long periods
//! without GC.
//!
//! Times are nanoseconds on any monotonic clock. `nprocs` is the number of CPUs
//! shared by the application and the GC (Go's `GOMAXPROCS`).
//!
//! Like the pacer, this depends only on `std`. Fields that Go protects with the
//! limiter's own lock are atomics here, accessed only while that lock is held.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};

use super::pacer::GC_BACKGROUND_UTILIZATION;

const ORD: Ordering = Ordering::Relaxed;

/// The maximum wall-clock time between limiter updates (10 ms).
// Go: runtime/mgclimit.go:140 (2c3c1caad2)
pub const GC_CPU_LIMITER_UPDATE_PERIOD: i64 = 10_000_000;

/// The bucket capacity per CPU (1 s in nanoseconds).
// Go: runtime/mgclimit.go:313 (2c3c1caad2)
pub const CAPACITY_PER_PROC: u64 = 1_000_000_000;

/// Go's GC CPU limiter. See the module documentation.
// Go: runtime/mgclimit.go:32-84 (2c3c1caad2)
#[derive(Debug, Default)]
pub struct CpuLimiter {
    lock: AtomicU32,

    enabled: AtomicBool,

    /// Whether the GC is in its mark phase (Go's copy of `gcBlackenEnabled`).
    /// Protected by `lock`.
    gc_enabled: AtomicBool,

    /// True while a STW pause transitions between mark and sweep. Protected by
    /// `lock`.
    transitioning: AtomicBool,

    /// Invariants: `fill <= capacity`. Protected by `lock`.
    bucket_fill: AtomicU64,
    bucket_capacity: AtomicU64,

    /// The cumulative GC CPU time that did not fit in the bucket. Protected by
    /// `lock`.
    overflow: AtomicU64,

    /// Assist time accumulated since the last update.
    assist_time_pool: AtomicI64,

    /// Idle time (including idle mark time) accumulated since the last update.
    idle_time_pool: AtomicI64,

    /// The time of the last update.
    last_update: AtomicI64,

    /// The GC cycle that last had the limiter enabled (Go: `numgc + 1` at the
    /// time).
    last_enabled_cycle: AtomicU32,

    /// The number of completed GC cycles, provided by the caller through
    /// `set_num_gc` (Go reads `memstats.numgc`).
    num_gc: AtomicU32,

    /// The number of CPUs (Go's copy of `gomaxprocs`). Protected by `lock`.
    nprocs: AtomicI32,
}

impl CpuLimiter {
    /// Create a limiter for `nprocs` CPUs, starting at time `now`.
    // Go: runtime/export_test.go:1602-1611 (2c3c1caad2)
    pub fn new(now: i64, nprocs: i32) -> Self {
        let l = Self::default();
        l.reset_capacity(now, nprocs);
        l
    }

    /// Whether the limiter is on, meaning the GC should limit its CPU use (by
    /// not running assists). Safe to call concurrently.
    // Go: runtime/mgclimit.go:86-92 (2c3c1caad2)
    pub fn limiting(&self) -> bool {
        self.enabled.load(ORD)
    }

    /// Notify the limiter of a GC transition: `enable_gc` is true when marking
    /// starts and false when it ends. Call in the STW pause, then call
    /// `finish_gc_transition` at the end of the pause. Between the two, the
    /// limiter cannot be updated otherwise.
    // Go: runtime/mgclimit.go:94-117 (2c3c1caad2)
    pub fn start_gc_transition(&self, enable_gc: bool, now: i64) {
        // This must happen during a STW, so we can't fail to acquire the lock.
        assert!(
            self.try_lock(),
            "failed to acquire lock to start a GC transition"
        );
        assert!(
            self.gc_enabled.load(ORD) != enable_gc,
            "transitioning GC to the same state as before?"
        );
        // Flush whatever was left between the last update and now.
        self.update_locked(now);
        self.gc_enabled.store(enable_gc, ORD);
        self.transitioning.store(true, ORD);
        // N.B. finishGCTransition releases the lock.
    }

    /// Finish a GC transition and release the limiter. The whole STW pause
    /// counts as GC time on all CPUs. `now` is the end of the pause.
    // Go: runtime/mgclimit.go:119-136 (2c3c1caad2)
    pub fn finish_gc_transition(&self, now: i64) {
        assert!(
            self.transitioning.load(ORD),
            "finishGCTransition called without starting one?"
        );
        // Count the full nprocs set of CPU time because the world is stopped
        // between startGCTransition and finishGCTransition. Even though the GC
        // isn't running on all CPUs, it is preventing user code from doing so,
        // so it might as well be.
        let last_update = self.last_update.load(ORD);
        if now >= last_update {
            self.accumulate(0, (now - last_update) * self.nprocs.load(ORD) as i64);
        }
        self.last_update.store(now, ORD);
        self.transitioning.store(false, ORD);
        self.unlock();
    }

    /// Whether the maximum update period has passed since the last update.
    // Go: runtime/mgclimit.go:142-146 (2c3c1caad2)
    pub fn need_update(&self, now: i64) -> bool {
        now - self.last_update.load(ORD) > GC_CPU_LIMITER_UPDATE_PERIOD
    }

    /// Add `t` ns of assist time, included in the next update.
    // Go: runtime/mgclimit.go:148-152 (2c3c1caad2)
    pub fn add_assist_time(&self, t: i64) {
        self.assist_time_pool.fetch_add(t, ORD);
    }

    /// Add `t` ns of time that a CPU was idle (or ran idle mark work),
    /// subtracted from the total CPU time in the next update.
    // Go: runtime/mgclimit.go:154-158 (2c3c1caad2)
    pub fn add_idle_time(&self, t: i64) {
        self.idle_time_pool.fetch_add(t, ORD);
    }

    /// Update the bucket at time `now`. Safe to call concurrently, except
    /// during a transition. If another update is running, this one is dropped
    /// (the other includes the pooled times).
    // Go: runtime/mgclimit.go:160-176 (2c3c1caad2)
    pub fn update(&self, now: i64) {
        if !self.try_lock() {
            // We failed to acquire the lock, which means something else is currently
            // updating. Just drop our update, the next one to update will include
            // our total assist time.
            return;
        }
        assert!(!self.transitioning.load(ORD), "update during transition");
        self.update_locked(now);
        self.unlock();
    }

    // Go: runtime/mgclimit.go:178-258 (2c3c1caad2)
    fn update_locked(&self, now: i64) {
        let last_update = self.last_update.load(ORD);
        if now < last_update {
            // Defensively avoid overflow. This isn't even the latest update anyway.
            return;
        }
        let mut window_total_time = (now - last_update) * self.nprocs.load(ORD) as i64;
        self.last_update.store(now, ORD);

        // Drain the pool of assist time.
        let assist_time = self.assist_time_pool.load(ORD);
        if assist_time != 0 {
            self.assist_time_pool.fetch_sub(assist_time, ORD);
        }

        // Drain the pool of idle time.
        let idle_time = self.idle_time_pool.load(ORD);
        if idle_time != 0 {
            self.idle_time_pool.fetch_sub(idle_time, ORD);
        }

        // Go: consuming the time of in-flight per-P limiter events is not
        // ported. Callers report assist and idle time when it ends. See the
        // deviations document.

        // Compute total GC time.
        let mut window_gc_time = assist_time;
        if self.gc_enabled.load(ORD) {
            window_gc_time += (window_total_time as f64 * GC_BACKGROUND_UTILIZATION) as i64;
        }

        // Subtract out all idle time from the total time. Do this after computing
        // GC time, because the background utilization is dependent on the *real*
        // total time, not the total time after idle time is subtracted.
        //
        // Idle time is counted as any time that a P is on the P idle list plus idle mark
        // time. Idle mark workers soak up time that the application spends idle.
        //
        // See Go's comment at runtime/mgclimit.go:241-254 on why this can turn
        // the limiter on even when it is not needed.
        window_total_time -= idle_time;

        self.accumulate(window_total_time - window_gc_time, window_gc_time);
    }

    /// Add time to the bucket and set whether the limiter is enabled. The lock
    /// must be held.
    // Go: runtime/mgclimit.go:260-297 (2c3c1caad2)
    fn accumulate(&self, mutator_time: i64, gc_time: i64) {
        let capacity = self.bucket_capacity.load(ORD);
        let fill = self.bucket_fill.load(ORD);
        let headroom = capacity - fill;
        let enabled = headroom == 0;

        // Let's be careful about three things here:
        // 1. The addition and subtraction, for the invariants.
        // 2. Overflow.
        // 3. Excessive mutation of l.enabled, which is accessed
        //    by all assists, potentially more than once.
        let change = gc_time - mutator_time;

        // Handle limiting case.
        if change > 0 && headroom <= change as u64 {
            self.overflow.fetch_add(change as u64 - headroom, ORD);
            self.bucket_fill.store(capacity, ORD);
            if !enabled {
                self.enabled.store(true, ORD);
                self.last_enabled_cycle
                    .store(self.num_gc.load(ORD).wrapping_add(1), ORD);
            }
            return;
        }

        // Handle non-limiting cases.
        if change < 0 && fill <= change.unsigned_abs() {
            // Bucket emptied.
            self.bucket_fill.store(0, ORD);
        } else {
            // All other cases.
            self.bucket_fill
                .store(fill.wrapping_sub((change.wrapping_neg()) as u64), ORD);
        }
        if change != 0 && enabled {
            self.enabled.store(false, ORD);
        }
    }

    // Go: runtime/mgclimit.go:299-302 (2c3c1caad2)
    fn try_lock(&self) -> bool {
        self.lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    // Go: runtime/mgclimit.go:304-310 (2c3c1caad2)
    fn unlock(&self) {
        let old = self.lock.swap(0, Ordering::Release);
        assert!(old == 1, "double unlock");
    }

    /// Set the capacity for `nprocs` CPUs. Must not be called while the GC is
    /// marking.
    // Go: runtime/mgclimit.go:315-338 (2c3c1caad2)
    pub fn reset_capacity(&self, now: i64, nprocs: i32) {
        // This must happen during a STW, so we can't fail to acquire the lock.
        assert!(self.try_lock(), "failed to acquire lock to reset capacity");
        // Flush the rest of the time for this period.
        self.update_locked(now);
        self.nprocs.store(nprocs, ORD);

        let capacity = nprocs as u64 * CAPACITY_PER_PROC;
        self.bucket_capacity.store(capacity, ORD);
        let fill = self.bucket_fill.load(ORD);
        if fill > capacity {
            self.bucket_fill.store(capacity, ORD);
            self.enabled.store(true, ORD);
            self.last_enabled_cycle
                .store(self.num_gc.load(ORD).wrapping_add(1), ORD);
        } else if fill < capacity {
            self.enabled.store(false, ORD);
        }
        self.unlock();
    }

    /// Set the number of completed GC cycles (Go's `memstats.numgc`), used
    /// only to record `last_enabled_cycle`.
    pub fn set_num_gc(&self, n: u32) {
        self.num_gc.store(n, ORD);
    }

    /// The GC cycle in which the limiter was last enabled.
    pub fn last_enabled_cycle(&self) -> u32 {
        self.last_enabled_cycle.load(ORD)
    }

    /// The bucket's fill (CPU-ns).
    pub fn fill(&self) -> u64 {
        self.bucket_fill.load(ORD)
    }

    /// The bucket's capacity (CPU-ns).
    pub fn capacity(&self) -> u64 {
        self.bucket_capacity.load(ORD)
    }

    /// The cumulative GC CPU time (CPU-ns) that overflowed the bucket.
    pub fn overflow(&self) -> u64 {
        self.overflow.load(ORD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MILLISECOND: i64 = 1_000_000;
    const MICROSECOND: i64 = 1_000;
    const SECOND: i64 = 1_000_000_000;
    const HOUR: i64 = 3600 * SECOND;

    // Go: runtime/mgclimit_test.go:13-255 (2c3c1caad2)
    #[test]
    fn gc_cpu_limiter() {
        const PROCS: i64 = 14;
        const CPP: i64 = CAPACITY_PER_PROC as i64;

        // Create mock time.
        let mut ticks: i64 = 0;
        let mut advance = |d: i64| -> i64 {
            ticks += d;
            ticks
        };

        // assistTime computes the CPU time for assists using frac of GOMAXPROCS
        // over the wall-clock duration d.
        let assist_time = |d: i64, frac: f64| -> i64 { (frac * d as f64 * PROCS as f64) as i64 };

        let l = CpuLimiter::new(0, PROCS as i32);

        // Do the whole test twice to make sure state doesn't leak across.
        let mut base_overflow: u64 = 0; // Track total overflow across iterations.
        for _ in 0..2 {
            assert_eq!(l.capacity(), (PROCS * CPP) as u64, "unexpected capacity");
            assert_eq!(l.fill(), 0, "expected empty bucket to start");

            // Test filling the bucket with just mutator time.

            l.update(advance(10 * MILLISECOND));
            l.update(advance(SECOND));
            l.update(advance(HOUR));
            assert_eq!(
                l.fill(),
                0,
                "expected empty bucket from only accumulating mutator time"
            );

            // Test needUpdate.

            assert!(
                !l.need_update(advance(GC_CPU_LIMITER_UPDATE_PERIOD / 2)),
                "need update even though updated half a period ago"
            );
            assert!(
                l.need_update(advance(GC_CPU_LIMITER_UPDATE_PERIOD)),
                "doesn't need update even though updated 1.5 periods ago"
            );
            l.update(advance(0));
            assert!(
                !l.need_update(advance(0)),
                "need update even though just updated"
            );

            // Test transitioning the bucket to enable the GC.

            l.start_gc_transition(true, advance(109 * MILLISECOND));
            l.finish_gc_transition(advance(2 * MILLISECOND + MICROSECOND));

            let expect = ((2 * MILLISECOND + MICROSECOND) * PROCS) as u64;
            assert_eq!(l.fill(), expect);

            // Test passing time without assists during a GC. Specifically, just enough to drain the bucket to
            // exactly procs nanoseconds (easier to get to because of rounding).
            //
            // The window we need to drain the bucket is 1/(1-2*gcBackgroundUtilization) times the current fill:
            //
            //   window = (fill - n) / (procs * (1-2*gcBackgroundUtilization)))
            //
            // And here we want n=procs:
            let factor = 1.0 / (1.0 - 2.0 * GC_BACKGROUND_UTILIZATION);
            let fill = (2 * MILLISECOND + MICROSECOND) * PROCS;
            l.update(advance(
                (factor * (fill - PROCS) as f64 / PROCS as f64) as i64,
            ));
            assert_eq!(
                l.fill(),
                PROCS as u64,
                "expected fill from draining after a GC started"
            );

            // Drain to zero for the rest of the test.
            l.update(advance(2 * PROCS * CPP));
            assert_eq!(l.fill(), 0, "expected empty bucket from draining");

            // Test filling up the bucket with 50% total GC work (so, not moving the bucket at all).
            l.add_assist_time(assist_time(
                10 * MILLISECOND,
                0.5 - GC_BACKGROUND_UTILIZATION,
            ));
            l.update(advance(10 * MILLISECOND));
            assert_eq!(l.fill(), 0, "expected empty bucket from 50% GC work");

            // Test adding to the bucket overall with 100% GC work.
            l.add_assist_time(assist_time(MILLISECOND, 1.0 - GC_BACKGROUND_UTILIZATION));
            l.update(advance(MILLISECOND));
            assert_eq!(
                l.fill(),
                (PROCS * MILLISECOND) as u64,
                "expected fill from 100% GC CPU"
            );
            assert!(
                !l.limiting(),
                "limiter is enabled after filling bucket but shouldn't be"
            );

            // Test filling the bucket exactly full.
            l.add_assist_time(assist_time(
                CPP - MILLISECOND,
                1.0 - GC_BACKGROUND_UTILIZATION,
            ));
            l.update(advance(CPP - MILLISECOND));
            assert_eq!(l.fill(), l.capacity(), "expected bucket filled to capacity");
            assert!(
                l.limiting(),
                "limiter is not enabled after filling bucket but should be"
            );
            assert_eq!(
                l.overflow(),
                base_overflow,
                "bucket filled exactly should not have overflow"
            );

            // Test adding with a delta of exactly zero. That is, GC work is exactly 50% of all resources.
            // Specifically, the limiter should still be on, and no overflow should accumulate.
            l.add_assist_time(assist_time(SECOND, 0.5 - GC_BACKGROUND_UTILIZATION));
            l.update(advance(SECOND));
            assert_eq!(l.fill(), l.capacity(), "expected bucket filled to capacity");
            assert!(
                l.limiting(),
                "limiter is not enabled after filling bucket but should be"
            );
            assert_eq!(
                l.overflow(),
                base_overflow,
                "bucket filled exactly should not have overflow"
            );

            // Drain the bucket by half.
            l.add_assist_time(assist_time(CPP, 0.0));
            l.update(advance(CPP));
            assert_eq!(l.fill(), l.capacity() / 2, "failed to drain by half");
            assert!(
                !l.limiting(),
                "limiter is enabled after draining bucket but shouldn't be"
            );

            // Test overfilling the bucket.
            l.add_assist_time(assist_time(CPP, 1.0 - GC_BACKGROUND_UTILIZATION));
            l.update(advance(CPP));
            assert_eq!(l.fill(), l.capacity(), "failed to fill to capacity");
            assert!(
                l.limiting(),
                "limiter is not enabled after overfill but should be"
            );
            let expect = (CPP * PROCS / 2) as u64;
            assert_eq!(
                l.overflow(),
                expect + base_overflow,
                "bucket overfilled should have overflow"
            );

            // Test ending the cycle with some assists left over.
            l.add_assist_time(assist_time(MILLISECOND, 1.0 - GC_BACKGROUND_UTILIZATION));
            l.start_gc_transition(false, advance(MILLISECOND));
            assert_eq!(
                l.fill(),
                l.capacity(),
                "failed to maintain fill to capacity"
            );
            assert!(
                l.limiting(),
                "limiter is not enabled after overfill but should be"
            );
            let expect = ((CPP / 2 + MILLISECOND) * PROCS) as u64;
            assert_eq!(
                l.overflow(),
                expect + base_overflow,
                "bucket overfilled should have overflow"
            );

            // Make sure the STW adds to the bucket.
            l.finish_gc_transition(advance(5 * MILLISECOND));
            assert_eq!(
                l.fill(),
                l.capacity(),
                "failed to maintain fill to capacity"
            );
            assert!(
                l.limiting(),
                "limiter is not enabled after overfill but should be"
            );
            let expect = ((CPP / 2 + 6 * MILLISECOND) * PROCS) as u64;
            assert_eq!(
                l.overflow(),
                expect + base_overflow,
                "bucket overfilled should have overflow"
            );

            // Resize procs up and make sure limiting stops.
            let expect_fill = l.capacity();
            l.reset_capacity(advance(0), (PROCS + 10) as i32);
            assert_eq!(
                l.fill(),
                expect_fill,
                "failed to maintain fill at old capacity"
            );
            assert!(
                !l.limiting(),
                "limiter is enabled after resetting capacity higher"
            );
            assert_eq!(
                l.overflow(),
                expect + base_overflow,
                "bucket overflow should have remained constant"
            );

            // Resize procs down and make sure limiting begins again.
            // Also make sure resizing doesn't affect overflow. This isn't
            // a case where we want to report overflow, because we're not
            // actively doing work to achieve it. It's that we have fewer
            // CPU resources now.
            l.reset_capacity(advance(0), (PROCS - 10) as i32);
            assert_eq!(l.fill(), l.capacity(), "failed lower fill to new capacity");
            assert!(
                l.limiting(),
                "limiter is disabled after resetting capacity lower"
            );
            assert_eq!(
                l.overflow(),
                expect + base_overflow,
                "bucket overflow should have remained constant"
            );

            // Get back to a zero state. The top of the loop will double check.
            l.reset_capacity(advance(CPP * PROCS), PROCS as i32);

            // Track total overflow for future iterations.
            base_overflow += ((CPP / 2 + 6 * MILLISECOND) * PROCS) as u64;
        }
    }
}
