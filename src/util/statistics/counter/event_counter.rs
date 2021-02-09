use super::*;
use crate::util::statistics::stats::{SharedStats, MAX_PHASES};
use std::sync::Arc;

/**
 * This file implements a simple event counter (counting number
 * events that occur for each phase).
 */
pub struct EventCounter {
    name: String,
    pub implicitly_start: bool,
    merge_phases: bool,
    count: Box<[u64; MAX_PHASES]>,
    current_count: u64,
    running: bool,
    stats: Arc<SharedStats>,
}

impl EventCounter {
    pub fn new(
        name: String,
        stats: Arc<SharedStats>,
        implicitly_start: bool,
        merge_phases: bool,
    ) -> Self {
        EventCounter {
            name,
            implicitly_start,
            merge_phases,
            count: box [0; MAX_PHASES],
            current_count: 0,
            running: false,
            stats,
        }
    }

    /**
     * Increment the event counter
     */
    pub fn inc(&mut self) {
        if self.running {
            self.inc_by(1);
        }
    }

    /**
     * Increment the event counter by provided value
     */
    pub fn inc_by(&mut self, value: u64) {
        if self.running {
            self.current_count += value;
        }
    }

    pub fn print_current(&self) {
        self.print_value(self.current_count);
    }

    fn print_value(&self, value: u64) {
        print!("{}", value);
    }
}

impl Counter for EventCounter {
    fn start(&mut self) {
        if !self.stats.get_gathering_stats() {
            return;
        }
        debug_assert!(!self.running);
        self.current_count = 0;
        self.running = true;
    }

    fn stop(&mut self) {
        if !self.stats.get_gathering_stats() {
            return;
        }
        debug_assert!(self.running);
        self.count[self.stats.get_phase()] = self.current_count;
        self.running = false;
    }

    /**
     * The phase has changed (from GC to mutator or mutator to GC).
     * Take action with respect to the last phase if necessary.
     */
    fn phase_change(&mut self, old_phase: usize) {
        if self.running {
            self.count[old_phase] = self.current_count;
            self.current_count = 0;
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
            None => {
                let mut total = 0;
                for p in 0..=self.stats.get_phase() {
                    total += self.count[p];
                }
                self.print_value(total);
            }
            Some(m) => {
                let mut total = 0;
                let mut p = if m { 0 } else { 1 };
                while p <= self.stats.get_phase() {
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
        while p < self.stats.get_phase() {
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
