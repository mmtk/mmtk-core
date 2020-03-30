// For each GC plan, the global Plan module may the same name as the GC plan,
// such as plan::nogc::nogc::NoGC, plan::g1::g1::G1. This is intentional.
#![allow(clippy::module_inception)]

pub mod collector_context;
pub mod controller_collector_context;
pub mod mutator_context;
pub mod parallel_collector;
mod parallel_collector_group;
pub mod phase;
pub mod plan;
pub mod plan_constraints;
mod trace;
pub mod tracelocal;
pub mod transitive_closure;

pub use self::collector_context::CollectorContext;
pub use self::mutator_context::MutatorContext;
pub use self::parallel_collector::ParallelCollector;
pub use self::parallel_collector_group::ParallelCollectorGroup;
pub use self::phase::Phase;
pub use self::plan::Allocator;
pub use self::plan::Plan;
pub use self::tracelocal::TraceLocal;
pub use self::transitive_closure::TransitiveClosure;

#[cfg(feature = "nogc")]
pub mod nogc;
#[cfg(feature = "nogc")]
pub use self::nogc as selected_plan;

#[cfg(feature = "semispace")]
pub mod semispace;
#[cfg(feature = "semispace")]
pub use self::semispace as selected_plan;

pub use self::selected_plan::SelectedConstraints;
pub use self::selected_plan::SelectedPlan;
