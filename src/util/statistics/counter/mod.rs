use std::time::Instant;

mod event_counter;
mod long_counter;
#[cfg(feature = "perf_counter")]
mod perf_event;
mod size_counter;

pub use self::event_counter::EventCounter;
pub use self::long_counter::{LongCounter, Timer};
#[cfg(feature = "perf_counter")]
pub use self::perf_event::PerfEventDiffable;
pub use self::size_counter::SizeCounter;

pub trait Counter {
    fn start(&mut self);
    fn stop(&mut self);
    fn phase_change(&mut self, old_phase: usize);
    fn print_count(&self, phase: usize);
    fn print_total(&self, mutator: Option<bool>);
    fn print_min(&self, mutator: bool);
    fn print_max(&self, mutator: bool);
    fn print_last(&self);
    fn merge_phases(&self) -> bool;
    fn implicitly_start(&self) -> bool;
    fn name(&self) -> &String;
}

/// A Diffable object could be stateless (e.g. a timer that reads the wall
/// clock), or stateful (e.g. holds reference to a perf event fd)
pub trait Diffable {
    type Val;
    fn current_value(&mut self) -> Self::Val;
    fn diff(&mut self, current: &Self::Val, earlier: &Self::Val) -> u64;
    fn print_diff(val: u64);
}

pub struct MonotoneNanoTime;

impl Diffable for MonotoneNanoTime {
    type Val = Instant;

    fn current_value(&mut self) -> Instant {
        Instant::now()
    }

    fn diff(&mut self, current: &Instant, earlier: &Instant) -> u64 {
        let delta = current.duration_since(*earlier);
        delta.as_secs() * 1_000_000_000 + u64::from(delta.subsec_nanos())
    }

    fn print_diff(val: u64) {
        print!("{}", format!("{:.*}", 2, val as f64 / 1e6f64));
    }
}
