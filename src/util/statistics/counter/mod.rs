use std::time::Instant;

mod event_counter;
mod long_counter;
mod size_counter;

pub use self::event_counter::EventCounter;
pub use self::long_counter::{LongCounter, Timer};
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
        delta.as_secs() * 1_000_000_000 + u64::from(delta.subsec_nanos())
    }

    fn print_diff(val: u64) {
        print!("{}", format!("{:.*}", 2, val as f64 / 1e6f64));
    }
}
