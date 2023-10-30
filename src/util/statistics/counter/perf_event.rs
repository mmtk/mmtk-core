use super::Diffable;
use pfm::{PerfEvent, PerfEventValue};

/// A [`Diffable`] helper type for measuring overall perf events for mutators
/// and GC
/// This is the process-wide counterpart of [`crate::scheduler::work_counter::WorkPerfEvent`].
pub struct PerfEventDiffable {
    pe: PerfEvent,
}

impl PerfEventDiffable {
    pub fn new(name: &str, exclude_kernel: bool) -> Self {
        let mut pe = PerfEvent::new(name, true)
            .unwrap_or_else(|_| panic!("Failed to create perf event {}", name));
        pe.set_exclude_kernel(exclude_kernel as u64);
        // measures the calling thread (and all child threads) on all CPUs
        pe.open(0, -1)
            .unwrap_or_else(|_| panic!("Failed to open perf event {}", name));
        PerfEventDiffable { pe }
    }
}

impl Diffable for PerfEventDiffable {
    type Val = PerfEventValue;

    fn start(&mut self) {
        self.pe.reset().expect("Failed to reset perf evet");
        self.pe.enable().expect("Failed to enable perf evet");
    }

    fn stop(&mut self) {
        self.pe.disable().expect("Failed to disable perf evet");
    }

    fn current_value(&mut self) -> Self::Val {
        let val = self.pe.read().expect("Failed to read perf evet");
        assert_eq!(val.time_enabled, val.time_running, "perf event multiplexed");
        val
    }

    fn diff(current: &Self::Val, earlier: &Self::Val) -> u64 {
        assert!(current.value >= earlier.value, "perf event overflowed");
        current.value as u64 - earlier.value as u64
    }

    fn print_diff(val: u64) {
        print!("{}", val);
    }
}
