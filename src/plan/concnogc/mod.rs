mod nogc;
mod nogcmutator;
mod nogctracelocal;
mod nogccollector;
pub mod nogcconstraints;

const VERBOSE: bool = false;

pub use self::nogc::NoGC;
pub use self::nogcmutator::NoGCMutator;
pub use self::nogctracelocal::NoGCTraceLocal;
pub use self::nogccollector::NoGCCollector;
pub use self::nogc::PLAN;

pub use self::nogc::SelectedPlan;
pub use self::nogcconstraints as SelectedConstraints;