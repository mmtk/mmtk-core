use super::*;
use crate::util::statistics::stats::SharedStats;
use std::sync::Arc;

pub struct SingleCounter {
    name: String,
    pub implicitly_start: bool,
    merge_phases: bool,
    current_count: u64,
    running: bool,
    stats: Arc<SharedStats>,
}

impl SingleCounter {
    pub fn new(
        name: String,
        stats: Arc<SharedStats>,
        implicitly_start: bool,
        merge_phases: bool,
    ) -> Self {
        SingleCounter {
            name,
            implicitly_start,
            merge_phases,
            current_count: 0,
            running: false,
            stats,
        }
    }

    pub fn set_count(&mut self, value: u64) {
        if self.running {
            self.current_count = value;
        }
    }

    pub fn print_current(&self) {
        self.print_value(self.current_count);
    }

    fn print_value(&self, value: u64) {
        print!("{}", value);
    }
}

impl Counter for SingleCounter {
    fn start(&mut self) {
        if !self.stats.get_gathering_stats() {
            return;
        }
        debug_assert!(!self.running);
        self.running = true;
    }

    fn stop(&mut self) {
        if !self.stats.get_gathering_stats() {
            return;
        }
        debug_assert!(self.running);
        self.running = false;
    }

    /**
     * The phase has changed (from GC to mutator or mutator to GC).
     * Take action with respect to the last phase if necessary.
     */
    fn phase_change(&mut self, _old_phase: usize) {}

    fn print_count(&self, _phase: usize) {
        self.print_value(self.current_count);
    }

    fn get_total(&self, _other: Option<bool>) -> u64 {
        self.current_count
    }

    fn print_total(&self, _mutator: Option<bool>) {
        self.print_value(self.current_count);
    }

    fn print_min(&self, _mutator: bool) {
        self.print_value(self.current_count);
    }

    fn print_max(&self, _mutator: bool) {
        self.print_value(self.current_count);
    }

    fn print_last(&self) {
        self.print_value(self.current_count);
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
