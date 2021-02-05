pub mod constraints;
mod gc_works;
mod global;
mod mutator;

pub use self::constraints as SelectedConstraints;
pub use self::global::GenCopy;
pub use self::global::SelectedPlan;

pub const FULL_NURSERY_GC: bool = true;
pub const NO_SLOW: bool = true;
