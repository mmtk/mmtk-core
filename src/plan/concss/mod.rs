mod ss;
mod sscollector;
mod ssmutator;
pub mod ssconstraints;
mod sstracelocal;
mod validate;

const VERBOSE: bool = false;//cfg!(debug_assertions);

pub use self::ss::SemiSpace;
pub use self::ss::PLAN;
pub use self::ssmutator::SSMutator;
pub use self::sstracelocal::SSTraceLocal;
pub use self::sscollector::SSCollector;

pub use self::ss::SelectedPlan;
pub use self::ssconstraints as SelectedConstraints;