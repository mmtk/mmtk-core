
#[macro_use] mod multitracelocal;
mod g1;
mod g1collector;
mod g1mutator;
pub mod g1constraints;
mod g1tracelocal;
mod g1marktracelocal;
mod g1evacuatetracelocal;
mod g1nurserytracelocal;
mod collection;
mod concurrent_refine;
mod validate;

pub use self::g1::G1;
pub use self::g1::PLAN;
pub use self::g1mutator::G1Mutator;
pub use self::g1marktracelocal::G1MarkTraceLocal;
pub use self::g1evacuatetracelocal::G1EvacuateTraceLocal;
pub use self::g1nurserytracelocal::G1NurseryTraceLocal;
pub use self::g1tracelocal::G1TraceLocal;
pub use self::g1collector::G1Collector;

pub use self::g1::SelectedPlan;
pub use self::g1constraints as SelectedConstraints;

const VERBOSE: bool = false;
const SLOW_ASSERTIONS: bool = false;

// Feature switches

const ENABLE_CONCURRENT_MARKING: bool = true;
const ENABLE_REMEMBERED_SETS: bool = true;
const ENABLE_CONCURRENT_REFINEMENT: bool = true;
const ENABLE_HOT_CARDS_OPTIMIZATION: bool = true;
const ENABLE_GENERATIONAL_GC: bool = true;

// Configs

const DIRTY_CARD_QUEUE_SIZE: usize = 500;
const CONCURRENT_REFINEMENT_THREADS: usize = 1;

