mod g1;
mod g1collector;
mod g1mutator;
pub mod g1constraints;
mod g1tracelocal;

pub use self::g1::G1;
pub use self::g1::PLAN;
pub use self::g1mutator::G1Mutator;
pub use self::g1tracelocal::G1TraceLocal;
pub use self::g1collector::G1Collector;

pub use self::g1::SelectedPlan;
pub use self::g1constraints as SelectedConstraints;

const DEBUG: bool = true;
