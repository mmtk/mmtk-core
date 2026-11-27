//! IncrementalMarkSweep: a non-moving mark-sweep collector with time-sliced incremental marking
//! paced to a mutator-utilization target. Based on Eclipse OMR/OpenJ9 Metronome GC.

pub(super) mod barrier;
pub(super) mod gc_work;
pub(super) mod global;
pub mod mutator;
pub(super) mod scheduler;

pub use global::IncrementalMarkSweep;
pub use global::INCREMENTAL_MARKSWEEP_CONSTRAINTS;
