use std::time::{Duration, Instant};

use super::stats::{get_gathering_stats, get_phase};

pub trait Counter {
    fn start(&mut self);
    fn stop(&mut self);
    fn phase_change(&mut self, old_phase: usize);
    fn print_count(&self, phase: usize);
    fn print_total(&self, mutator: Option<bool>);
    fn print_min(&self, mutator: bool);
    fn print_max(&self, mutator: bool);
    fn print_last(&self) {
        let phase = get_phase();
        if phase > 0 {
            self.print_count(phase - 1);
        }
    }
    fn merge_phases(&self) -> bool;
}

pub trait Diffable {
    type Val;
    fn current_value() -> Self::Val;
    fn diff(current: &Self::Val, earlier: &Self::Val) -> u64;
    fn print_diff(val: u64);
}

pub struct MonotoneNanoTime;

impl Diffable for MonotoneNanoTime {
    type Val = Instant;

    fn current_value() -> Instant {
        Instant::now()
    }

    fn diff(current: &Instant, earlier: &Instant) -> u64 {
        let delta = current.duration_since(*earlier);
        delta.as_secs() * 1_000_000_000 + delta.subsec_nanos() as u64
    }

    fn print_diff(val: u64) {
        print!("{}", format!("{:.*}", 2, val as f64 / 1e6f64));
    }
}

pub struct LongCounter<T: Diffable> {
    name: String,
    pub implicitly_start: bool,
    merge_phases: bool,
    count: [u64; super::stats::MAX_PHASES], // FIXME make this resizable
    start_value: Option<T::Val>,
    total_count: u64,
    running: bool
}

impl<T: Diffable> Counter for LongCounter<T> {
    fn start(&mut self) {
        if !get_gathering_stats() {
            return;
        }
        debug_assert!(!self.running);
        self.running = true;
        self.start_value = Some(T::current_value());
    }

    fn stop(&mut self) {
        if !get_gathering_stats() {
            return;
        }
        debug_assert!(self.running);
        self.running = false;
        let delta = T::diff(&T::current_value(), self.start_value.as_ref().unwrap());
        self.count[get_phase()] += delta;
        self.total_count += delta;
    }

    fn phase_change(&mut self, old_phase: usize) {
        if self.running {
            let now = T::current_value();
            let delta = T::diff(&now, self.start_value.as_ref().unwrap());
            self.count[old_phase] += delta;
            self.total_count += delta;
            self.start_value = Some(now);
        }
    }

    fn print_count(&self, phase: usize) {
        if self.merge_phases() {
            debug_assert!((phase | 1) == (phase + 1));
            self.print_value(self.count[phase] + self.count[phase + 1]);
        } else {
            self.print_value(self.count[phase]);
        }
    }

    fn print_total(&self, mutator: Option<bool>) {
        match mutator {
            None => { self.print_value(self.total_count) },
            Some(m) => {
                let mut total = 0;
                let mut p = if m { 0 } else { 1 };
                while p <= get_phase() {
                    total += self.count[p];
                    p += 2;
                }
                self.print_value(total);
            }
        };
    }

    fn print_min(&self, mutator: bool) {
        let mut p = if mutator { 0 } else { 1 };
        let mut min = self.count[p];
        while p < get_phase() {
            if self.count[p] < min {
                min = self.count[p];
                p += 2;
            }
        }
        self.print_value(min);
    }

    fn print_max(&self, mutator: bool) {
        let mut p = if mutator { 0 } else { 1 };
        let mut max = self.count[p];
        while p < get_phase() {
            if self.count[p] > max {
                max = self.count[p];
                p += 2;
            }
        }
        self.print_value(max);
    }

    fn merge_phases(&self) -> bool {
        self.merge_phases
    }
}

impl<T: Diffable> LongCounter<T>{
    pub fn new(name: String, implicitly_start: bool, merge_phases: bool) -> Self {
        LongCounter {
            name,
            implicitly_start,
            merge_phases,
            count: [0; super::stats::MAX_PHASES],
            start_value: None,
            total_count: 0,
            running: false
        }
    }

    pub fn print_value(&self, val: u64) {
        T::print_diff(val);
    }
}

pub type Timer = LongCounter<MonotoneNanoTime>;