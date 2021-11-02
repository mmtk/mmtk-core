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

/// An abstraction over how a specific Diffable value is counted
///
/// For example, we can just collect the values, and store the cummulative sum,
/// or we can derive some kind of histogram, etc.
pub trait Counter {
    /// Start the counter
    fn start(&mut self);
    /// Stop the counter
    fn stop(&mut self);
    /// Signal a change in GC phase.
    ///
    /// The phase number starts from 0 and is strictly increasing.
    /// Even numbers mean mutators are running (`other`) while odd numbers mean
    /// stop-the-world pauses (`stw`).
    /// Take action with respect to the last phase if necessary.
    fn phase_change(&mut self, old_phase: usize);
    /// Print the counter value for a particular phase
    ///
    /// If the counter merges the phases, the printing value will include
    /// the specified phase and the next phase
    fn print_count(&self, phase: usize);
    /// Get the total count over past phases
    ///
    /// If the argument is None, count all phases.
    /// Otherwise, count only `other` phases if true, or `stw` phases if false
    fn get_total(&self, other: Option<bool>) -> u64;
    /// Print the total count over past phases
    ///
    /// If the argument is None, count all phases.
    /// Otherwise, count only `other` phases if true, or `stw` phases if false
    fn print_total(&self, other: Option<bool>);
    /// Print the minimum count of the past phases
    ///
    /// Consider only `other` phases if true, or `stw` phases if false
    fn print_min(&self, other: bool);
    /// Print the maximum count of the past phases
    ///
    /// Consider only `other` phases if true, or `stw` phases if false
    fn print_max(&self, other: bool);
    /// Print the count of the last phases
    fn print_last(&self);
    /// Whether the counter merges other and stw phases.
    fn merge_phases(&self) -> bool;
    /// Whether the counter starts implicitly after creation
    ///
    /// FIXME currently unused
    fn implicitly_start(&self) -> bool;
    /// Get the name of the counter
    fn name(&self) -> &String;
}

/// An abstraction over some changing values that we want to measure.
///
/// A Diffable object could be stateless (e.g. a timer that reads the wall
/// clock), or stateful (e.g. holds reference to a perf event fd)
pub trait Diffable {
    /// The type of each reading
    type Val;
    /// Start the Diffable
    fn start(&mut self);
    /// Stop the Diffable
    fn stop(&mut self);
    /// Read the current value
    fn current_value(&mut self) -> Self::Val;
    /// Compute the difference between two readings
    fn diff(current: &Self::Val, earlier: &Self::Val) -> u64;
    /// Print the difference in a specific format
    fn print_diff(val: u64);
}

pub struct MonotoneNanoTime;

impl Diffable for MonotoneNanoTime {
    type Val = Instant;

    /// nop for the wall-clock time
    fn start(&mut self) {}

    /// nop for the wall-clock time
    fn stop(&mut self) {}

    fn current_value(&mut self) -> Instant {
        Instant::now()
    }

    fn diff(current: &Instant, earlier: &Instant) -> u64 {
        let delta = current.duration_since(*earlier);
        delta.as_secs() * 1_000_000_000 + u64::from(delta.subsec_nanos())
    }

    fn print_diff(val: u64) {
        print!("{:.*}", 2, val as f64 / 1e6f64);
    }
}
