
#[macro_use] mod multitracelocal;
mod g1;
mod g1collector;
mod g1mutator;
pub mod g1constraints;
mod g1tracelocal;
mod g1marktracelocal;
mod g1evacuatetracelocal;
mod collection;
mod concurrent_refine;
mod validate;

pub use self::g1::G1;
pub use self::g1::PLAN;
pub use self::g1mutator::G1Mutator;
pub use self::g1marktracelocal::G1MarkTraceLocal;
pub use self::g1evacuatetracelocal::G1EvacuateTraceLocal;
pub use self::g1tracelocal::G1TraceLocal;
pub use self::g1collector::G1Collector;

pub use self::g1::SelectedPlan;
pub use self::g1constraints as SelectedConstraints;

const VERBOSE: bool = true;

// Feature switches

const ENABLE_CONCURRENT_MARKING: bool = false;
const ENABLE_FULL_TRACE_EVACUATION: bool = true;
const ENABLE_GENERATIONAL_GC: bool = false;

// Configs

const DIRTY_CARD_QUEUE_SIZE: usize = 500;
const CONCURRENT_REFINEMENT_THREADS: usize = 1;

// Derived

const USE_REMEMBERED_SETS: bool = !ENABLE_FULL_TRACE_EVACUATION;
const USE_CARDS: bool = USE_REMEMBERED_SETS;

