use super::Diffable;
use pfm::{PerfEvent, PerfEventValue};

/// A [`Diffable`] helper type for measuring overall perf events for mutators
/// and GC
/// This is the process-wide counterpart of [`crate::scheduler::work_counter::WorkPerfEvent`].
pub struct PerfEventDiffable {
    pe: PerfEvent,
}

impl PerfEventDiffable {
    pub fn new(name: &str) -> Self {
        let mut pe = PerfEvent::new(name, true)
            .unwrap_or_else(|_| panic!("Failed to create perf event {}", name));
        // measures the calling thread (and all child threads) on all CPUs
        pe.open(0, -1)
            .unwrap_or_else(|_| panic!("Failed to open perf event {}", name));
        PerfEventDiffable { pe }
    }
}

impl Diffable for PerfEventDiffable {
    type Val = PerfEventValue;

    fn current_value(&mut self) -> Self::Val {
        let val = self.pe.read().unwrap();
        self.pe.enable();
        self.pe.reset();
        val
    }

    fn diff(current: &Self::Val, _earlier: &Self::Val) -> u64 {
        // earlier value is not used as the counter is reset after each use
        assert_eq!(current.time_enabled, current.time_running);
        current.value as u64
    }

    fn print_diff(val: u64) {
        print!("{}", val);
    }
}
