//! IncrementalMarkSweep time-based scheduling.

use crate::util::deadline::now_nanos;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const MAX_SLICES: usize = 100;

#[derive(Clone, Copy, Debug)]
struct Slice {
    /// Duration in seconds.
    duration: f64,
    mutator: bool,
}

/// Sliding window of mutator/GC time slices.
#[derive(Debug)]
pub struct UtilizationTracker {
    window: f64,
    target: f64,
    beat_nanos: u64,
    slices: Vec<Slice>,
    last_nanos: u64,
    current_utilization: f64,
    nanos_left_in_slice: u64,
}

impl UtilizationTracker {
    pub fn new(window: Duration, target: f64, beat: Duration) -> Self {
        let window_s = window.as_secs_f64();
        Self {
            window: window_s,
            target,
            beat_nanos: beat.as_nanos() as u64,
            slices: vec![Slice {
                duration: window_s,
                mutator: true,
            }],
            last_nanos: now_nanos(),
            current_utilization: 1.0,
            nanos_left_in_slice: beat.as_nanos() as u64,
        }
    }

    pub fn target(&self) -> f64 {
        self.target
    }

    /// Record that the time since the last call was spent by the mutator (`is_mutator`) or the
    /// GC.
    pub fn add_time_slice(&mut self, mut is_mutator: bool) {
        let now = now_nanos();
        let mut elapsed = if now >= self.last_nanos {
            (now - self.last_nanos) as f64 / 1e9
        } else {
            // Clock went backwards: charge a GC slice of `window * (1 - target)`.
            is_mutator = false;
            self.window * (1.0 - self.target)
        };
        if elapsed > self.window {
            elapsed = self.window;
        }
        self.last_nanos = now;
        self.slices.push(Slice {
            duration: elapsed,
            mutator: is_mutator,
        });

        // Slide the window: drop `elapsed` seconds from the oldest end.
        let mut remaining = elapsed;
        let mut i = 0;
        while i < self.slices.len() - 1 && remaining > self.slices[i].duration {
            remaining -= self.slices[i].duration;
            i += 1;
        }
        self.slices[i].duration -= remaining;
        if self.slices[i].duration < 0.0 {
            self.slices[i].duration = 0.0;
        }
        self.slices.drain(0..i);

        let mutator_sum: f64 = self
            .slices
            .iter()
            .filter(|s| s.mutator)
            .map(|s| s.duration)
            .sum();
        if self.slices.len() >= MAX_SLICES {
            // Compact to two slices.
            self.slices = vec![
                Slice {
                    duration: mutator_sum,
                    mutator: true,
                },
                Slice {
                    duration: (self.window - mutator_sum).max(0.0),
                    mutator: false,
                },
            ];
        } else {
            // Renormalise round-off into the oldest slice.
            let total: f64 = self.slices.iter().map(|s| s.duration).sum();
            self.slices[0].duration += self.window - total;
            if self.slices[0].duration < 0.0 {
                self.slices[0].duration = 0.0;
            }
        }
        self.current_utilization = (mutator_sum / self.window).clamp(0.0, 1.0);

        let excess = (self.current_utilization - self.target) * self.window * 1e9;

        self.nanos_left_in_slice = if excess < 0.0 {
            self.beat_nanos
        } else {
            u64::min(excess as u64, self.beat_nanos)
        };
    }

    /// Length of a GC slice that starts now: `min(excess, beat)`.
    pub fn nanos_left_in_slice(&self) -> u64 {
        self.nanos_left_in_slice
    }

    /// `(util - target) * window / beat`, the "excess" measured in beats.
    pub fn excess_beats(&self) -> f64 {
        (self.current_utilization - self.target) * self.window * 1e9 / self.beat_nanos as f64
    }
}

/// Shared timing state of an IncrementalMarkSweep plan.
pub struct IncrementalMarkSweepTimer {
    pub beat: Duration,
    pub tracker: Mutex<UtilizationTracker>,
    /// When the current mutator slice started.
    pub mutator_slice_start: AtomicU64,
    /// GC beats in the current increment.
    consecutive_gc_beats: AtomicU64,
}

impl IncrementalMarkSweepTimer {
    pub fn new(beat: Duration, window: Duration, target: f64) -> Self {
        Self {
            beat,
            tracker: Mutex::new(UtilizationTracker::new(window, target, beat)),
            mutator_slice_start: AtomicU64::new(now_nanos()),
            consecutive_gc_beats: AtomicU64::new(0),
        }
    }

    /// Charge the time up to the stop request to the mutator. The time to stop the mutators
    /// goes to the next GC slice.
    pub fn on_stop_mutators(&self) {
        self.tracker.lock().unwrap().add_time_slice(true);
    }

    /// Start a GC slice once mutators are stopped. Returns its length.
    pub fn start_gc_increment(&self) -> Duration {
        self.consecutive_gc_beats.store(1, Ordering::SeqCst);
        self.start_gc_slice()
    }

    pub fn start_gc_double_beat(&self) -> Duration {
        self.consecutive_gc_beats.fetch_add(1, Ordering::SeqCst);
        self.start_gc_slice()
    }

    fn start_gc_slice(&self) -> Duration {
        let mut t = self.tracker.lock().unwrap();
        t.add_time_slice(false);
        Duration::from_nanos(t.nanos_left_in_slice())
    }

    /// GC slice ended. Mutators start a new slice.
    pub fn on_pause_end(&self) {
        self.tracker.lock().unwrap().add_time_slice(false);
        self.mutator_slice_start
            .store(now_nanos(), Ordering::SeqCst);
    }

    /// Called by the alarm thread when a mutator beat has expired.
    /// Returns `true` if the mutator should get another beat (no GC now).
    pub fn should_mutator_double_beat(&self) -> bool {
        let mut t = self.tracker.lock().unwrap();
        t.add_time_slice(true);
        t.excess_beats() <= 1.0
    }

    /// Returns `true` if the GC may keep the mutators paused for another slice.
    pub fn should_gc_double_beat(&self) -> bool {
        let mut t = self.tracker.lock().unwrap();
        let target = t.target();
        if target <= 0.0 {
            return true;
        }
        let max_consecutive_beats = (1.0 / target) as u64;
        if self.consecutive_gc_beats.load(Ordering::SeqCst) >= max_consecutive_beats {
            return false;
        }
        t.add_time_slice(false);
        t.excess_beats() >= 2.0
    }

    /// Alarm thread period.
    pub fn alarm_period(&self) -> Duration {
        let third = self.beat / 3;
        if self.beat > Duration::from_micros(1000) {
            Duration::max(third, Duration::from_micros(1000))
        } else {
            third
        }
    }
}

/// Alarm thread state.
#[derive(Default)]
pub struct AlarmThread {
    pub running: AtomicBool,
    pub handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}
