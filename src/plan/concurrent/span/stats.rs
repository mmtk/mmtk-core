//! Statistics for evaluating `ConcurrentSpan` (design §10.1).  They are printed with MMTk's
//! other statistics between `harness_begin` and `harness_end`.

use crate::plan::concurrent::Pause;
use crate::util::statistics::counter::EventCounter;
use crate::util::statistics::stats::Stats;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Counter = Arc<Mutex<EventCounter>>;

pub(super) struct SpanStats {
    initial_mark_pauses: Counter,
    initial_mark_pause_ns: Counter,
    final_mark_pauses: Counter,
    final_mark_pause_ns: Counter,
    full_pauses: Counter,
    full_pause_ns: Counter,
    ragged_rounds: Counter,
    mark_restarts: Counter,
    stalls: Counter,
    stall_ns: Counter,
    assists: Counter,
    assists_parked: Counter,
    assists_limited: Counter,
    sweep_residue_pages: Counter,
    /// When the current pause started.
    pause_start: Mutex<Option<Instant>>,
}

impl SpanStats {
    pub(super) fn new(stats: &Stats) -> Self {
        let counter = |name: &str| stats.new_event_counter(name, true, true);
        Self {
            initial_mark_pauses: counter("span.initialMark"),
            initial_mark_pause_ns: counter("span.initialMark.ns"),
            final_mark_pauses: counter("span.finalMark"),
            final_mark_pause_ns: counter("span.finalMark.ns"),
            full_pauses: counter("span.full"),
            full_pause_ns: counter("span.full.ns"),
            ragged_rounds: counter("span.raggedRounds"),
            mark_restarts: counter("span.markRestarts"),
            stalls: counter("span.stalls"),
            stall_ns: counter("span.stall.ns"),
            assists: counter("span.assists"),
            assists_parked: counter("span.assistsParked"),
            assists_limited: counter("span.assistsLimited"),
            sweep_residue_pages: counter("span.sweepResiduePages"),
            pause_start: Mutex::new(None),
        }
    }

    fn inc(counter: &Counter, by: u64) {
        counter.lock().unwrap().inc_by(by);
    }

    /// Called when the mutators have stopped for a pause.
    pub(super) fn pause_started(&self) {
        *self.pause_start.lock().unwrap() = Some(Instant::now());
    }

    /// Called before the mutators resume after a pause.  The time between the two calls is a
    /// little shorter than the time mutators are stopped.
    pub(super) fn pause_ended(&self, pause: Pause) {
        let Some(start) = self.pause_start.lock().unwrap().take() else {
            return;
        };
        let ns = start.elapsed().as_nanos() as u64;
        let (count, time) = match pause {
            Pause::InitialMark => (&self.initial_mark_pauses, &self.initial_mark_pause_ns),
            Pause::FinalMark => (&self.final_mark_pauses, &self.final_mark_pause_ns),
            Pause::Full => (&self.full_pauses, &self.full_pause_ns),
            Pause::RefCount => return,
        };
        Self::inc(count, 1);
        Self::inc(time, ns);
    }

    pub(super) fn ragged_round(&self) {
        Self::inc(&self.ragged_rounds, 1);
    }

    pub(super) fn mark_restart(&self) {
        Self::inc(&self.mark_restarts, 1);
    }

    pub(super) fn stall(&self) {
        Self::inc(&self.stalls, 1);
    }

    pub(super) fn stall_finished(&self, duration: Duration) {
        Self::inc(&self.stall_ns, duration.as_nanos() as u64);
    }

    pub(super) fn assist(&self, parked: bool) {
        Self::inc(&self.assists, 1);
        if parked {
            Self::inc(&self.assists_parked, 1);
        }
    }

    pub(super) fn assist_limited(&self) {
        Self::inc(&self.assists_limited, 1);
    }

    pub(super) fn sweep_residue(&self, pages: usize) {
        Self::inc(&self.sweep_residue_pages, pages as u64);
    }
}
