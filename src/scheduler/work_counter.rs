//! Counter for work packets
//!
//! Provides an abstraction and implementations of counters for collecting
//! work-packet level statistics
//!
//! See [`crate::util::statistics`] for collecting statistics over a GC cycle
use std::time::SystemTime;

/// Common struct for different work counters
///
/// Stores the total, min and max of counter readings
#[derive(Copy, Clone, Debug)]
pub(super) struct WorkCounterBase {
    pub(super) total: f64,
    pub(super) min: f64,
    pub(super) max: f64,
}

/// Make [`WorkCounter`] trait objects cloneable
pub(super) trait WorkCounterClone {
    /// Clone the object
    fn clone_box(&self) -> Box<dyn WorkCounter>;
}

impl<T: 'static + WorkCounter + Clone> WorkCounterClone for T {
    fn clone_box(&self) -> Box<dyn WorkCounter> {
        Box::new(self.clone())
    }
}

/// An abstraction of work counters
///
/// Use for trait objects, as we have might have types of work counters for
/// the same work packet and the types are not statically known.
/// The overhead should be negligible compared with the cost of executing
/// a work packet.
pub(super) trait WorkCounter: WorkCounterClone + std::fmt::Debug {
    // TODO: consolidate with crate::util::statistics::counter::Counter;
    /// Start the counter
    fn start(&mut self);
    /// Stop the counter
    fn stop(&mut self);
    /// Name of counter
    fn name(&self) -> String;
    /// Return a reference to [`WorkCounterBase`]
    fn get_base(&self) -> &WorkCounterBase;
    /// Return a mutatable reference to [`WorkCounterBase`]
    fn get_base_mut(&mut self) -> &mut WorkCounterBase;
}

impl Clone for Box<dyn WorkCounter> {
    fn clone(&self) -> Box<dyn WorkCounter> {
        self.clone_box()
    }
}

impl Default for WorkCounterBase {
    fn default() -> Self {
        WorkCounterBase {
            total: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl WorkCounterBase {
    /// Merge two [`WorkCounterBase`], keep the semantics of the fields,
    /// and return a new object
    pub(super) fn merge(&self, other: &Self) -> Self {
        let min = self.min.min(other.min);
        let max = self.max.max(other.max);
        let total = self.total + other.total;
        WorkCounterBase { total, min, max }
    }

    /// Merge two [`WorkCounterBase`], modify the current object in place,
    /// and keep the semantics of the fields
    pub(super) fn merge_inplace(&mut self, other: &Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
        self.total += other.total;
    }

    /// Update the object based on a single value
    pub(super) fn merge_val(&mut self, val: f64) {
        self.min = self.min.min(val);
        self.max = self.max.max(val);
        self.total += val;
    }
}

/// Measure the durations of work packets
///
/// Timing is based on [`SystemTime`]
#[derive(Copy, Clone, Debug)]
pub(super) struct WorkDuration {
    base: WorkCounterBase,
    start_value: Option<SystemTime>,
    running: bool,
}

impl WorkDuration {
    pub(super) fn new() -> Self {
        WorkDuration {
            base: Default::default(),
            start_value: None,
            running: false,
        }
    }
}

impl WorkCounter for WorkDuration {
    fn start(&mut self) {
        self.start_value = Some(SystemTime::now());
        self.running = true;
    }

    fn stop(&mut self) {
        let duration = self.start_value.unwrap().elapsed().unwrap().as_nanos() as f64;
        self.base.merge_val(duration);
    }

    fn name(&self) -> String {
        "time".to_owned()
    }

    fn get_base(&self) -> &WorkCounterBase {
        &self.base
    }

    fn get_base_mut(&mut self) -> &mut WorkCounterBase {
        &mut self.base
    }
}
