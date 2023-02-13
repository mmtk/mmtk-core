use super::*;
use crate::util::statistics::stats::{SharedStats, MAX_PHASES};
use std::fmt;
use std::sync::Arc;

pub struct LongCounter<T: Diffable> {
    name: String,
    pub implicitly_start: bool,
    merge_phases: bool,
    count: Box<[u64; MAX_PHASES]>, // FIXME make this resizable
    diffable: T,
    start_value: Option<T::Val>,
    total_count: u64,
    running: bool,
    stats: Arc<SharedStats>,
}

impl<T: Diffable> fmt::Debug for LongCounter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LongCounter({})", self.name)
    }
}

impl<T: Diffable> Counter for LongCounter<T> {
    fn start(&mut self) {
        if !self.stats.get_gathering_stats() {
            return;
        }
        debug_assert!(!self.running);
        self.running = true;
        self.diffable.start();
        self.start_value = Some(self.diffable.current_value());
    }

    fn stop(&mut self) {
        if !self.stats.get_gathering_stats() {
            return;
        }
        debug_assert!(self.running);
        self.running = false;
        self.diffable.stop();
        let current_value = self.diffable.current_value();
        let delta = T::diff(&current_value, self.start_value.as_ref().unwrap());
        self.count[self.stats.get_phase()] += delta;
        self.total_count += delta;
    }

    fn phase_change(&mut self, old_phase: usize) {
        if self.running {
            let now = self.diffable.current_value();
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

    fn get_total(&self, other: Option<bool>) -> u64 {
        match other {
            None => self.total_count,
            Some(m) => {
                let mut total = 0;
                let mut p = !m as usize;
                while p <= self.stats.get_phase() {
                    total += self.count[p];
                    p += 2;
                }
                total
            }
        }
    }

    fn print_total(&self, other: Option<bool>) {
        self.print_value(self.get_total(other));
    }

    fn print_min(&self, other: bool) {
        let mut p = !other as usize;
        let mut min = self.count[p];
        while p < self.stats.get_phase() {
            if self.count[p] < min {
                min = self.count[p];
                p += 2;
            }
        }
        self.print_value(min);
    }

    fn print_max(&self, other: bool) {
        let mut p = !other as usize;
        let mut max = self.count[p];
        while p < self.stats.get_phase() {
            if self.count[p] > max {
                max = self.count[p];
                p += 2;
            }
        }
        self.print_value(max);
    }

    fn print_last(&self) {
        let phase = self.stats.get_phase();
        if phase > 0 {
            self.print_count(phase - 1);
        }
    }

    fn merge_phases(&self) -> bool {
        self.merge_phases
    }

    fn implicitly_start(&self) -> bool {
        self.implicitly_start
    }

    fn name(&self) -> &String {
        &self.name
    }
}

impl<T: Diffable> LongCounter<T> {
    pub fn new(
        name: String,
        stats: Arc<SharedStats>,
        implicitly_start: bool,
        merge_phases: bool,
        diffable: T,
    ) -> Self {
        LongCounter {
            name,
            implicitly_start,
            merge_phases,
            count: Box::new([0; MAX_PHASES]),
            diffable,
            start_value: None,
            total_count: 0,
            running: false,
            stats,
        }
    }

    fn print_value(&self, val: u64) {
        T::print_diff(val);
    }
}

pub type Timer = LongCounter<MonotoneNanoTime>;
