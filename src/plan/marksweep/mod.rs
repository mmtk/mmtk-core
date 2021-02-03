mod gc_works;
mod global;
pub mod malloc;
pub mod metadata;
pub mod mutator;

pub use self::global::MarkSweep;

pub use self::global::SelectedPlan;
