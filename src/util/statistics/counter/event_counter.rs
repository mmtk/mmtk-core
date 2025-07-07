use super::*;
use crate::util::statistics::stats::{SharedStats, DEFAULT_NUM_PHASES};
use std::sync::Arc;

/**
 * This file implements a simple event counter (counting number
 * events that occur for each phase).
 */
pub struct EventCounter {
    name: String,
    pub implicitly_start: bool,
    merge_phases: bool,
    count: Vec<u64>,
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
            count: Vec::with_capacity(DEFAULT_NUM_PHASES),
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
        print!("{value}");
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
        self.count.push(self.current_count);
        debug_assert_eq!(self.count[self.stats.get_phase()], self.current_count);
        self.running = false;
    }

    fn phase_change(&mut self, old_phase: usize) {
        if self.running {
            self.count.push(self.current_count);
            debug_assert_eq!(self.count[old_phase], self.current_count);
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

    fn get_total(&self, other: Option<bool>) -> u64 {
        match other {
            None => {
                let mut total = 0;
                for p in 0..=self.stats.get_phase() {
                    total += self.count[p];
                }
                total
            }
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
