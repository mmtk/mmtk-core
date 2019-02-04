use std::time::{Duration, Instant};

use super::stats::{get_gathering_stats, get_phase};

pub trait Counter {
    fn start(&mut self);
    fn stop(&mut self);
    fn phase_change(&mut self, old_phase: usize);
    fn print_count(&self, phase: usize);
    fn print_total(&self, mutator: bool);
    fn print_min(&self, mutator: bool);
    fn print_max(&self, mutator: bool);
    fn print_last(&self) {
        let phase = get_phase();
        if phase > 0 {
            self.print_count(phase - 1);
        }
    }
}

trait Diffiable {
    type Val;
    fn current_value() -> Self::Val;
    fn diff(val: &Self::Val) -> u64;
}

struct MonotoneNanoTime;

impl Diffiable for MonotoneNanoTime {
    type Val = Instant;

    fn current_value() -> Instant {
        Instant::now()
    }

    fn diff(val: &Instant) -> u64 {
        let now = Instant::now();
        let delta = now.duration_since(*val);
        delta.as_secs() * 1_000_000_000 + delta.subsec_nanos() as u64
    }
}

struct LongCounter<T: Diffiable> {
    count: [u64; super::stats::MAX_PHASES],
    start_value: T::Val,
    total_count: u64,
    running: bool,
}

impl<T: Diffiable> Counter for LongCounter<T> {
    fn start(&mut self) {
        if !get_gathering_stats() {
            return;
        }
        debug_assert!(!self.running);
        self.running = true;
        self.start_value = T::current_value();
    }

    fn stop(&mut self) {
        if !get_gathering_stats() {
            return;
        }
        debug_assert!(self.running);
        self.running = false;
        let delta = T::diff(&self.start_value);
        self.count[get_phase()] += delta;
        self.total_count += delta;
    }

    fn phase_change(&mut self, old_phase: usize) {
        unimplemented!()
    }

    fn print_count(&self, phase: usize) {
        unimplemented!()
    }

    fn print_total(&self, mutator: bool) {
        unimplemented!()
    }

    fn print_min(&self, mutator: bool) {
        unimplemented!()
    }

    fn print_max(&self, mutator: bool) {
        unimplemented!()
    }
}

pub type Timer = LongCounter<MonotoneNanoTime>;