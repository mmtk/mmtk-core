//! GC algorithms from the MMTk suite.
//!
//! This module provides various GC plans, each of which implements a GC algorithm.
//! Generally a plan consists of a few parts:
//! * A plan type that implements the [`Plan`](crate::plan::global::Plan) trait, which defines
//!   spaces used in the plan, and their behaviors in GC and page accounting.
//! * A mutator definition, which describes the mapping between allocators and allocation semantics,
//!   and the mapping between allocators and spaces. If the plan needs barrier, the barrier definition is
//!   also included here.
//! * A constant for [`PlanConstraints`](crate::plan::plan_constraints::PlanConstraints), which defines
//!   plan-specific constants.
//! * Plan-specific [`GCWork`](crate::scheduler::GCWork), which is scheduled during GC. If the plan
//!   implements a copying GC, a [`CopyContext`](crate::plan::global::CopyContext) also needs to be provided.
//!
//! For more about implementing a plan, it is recommended to read the [MMTk tutorial](/docs/tutorial/Tutorial.md).

pub mod barriers;
pub mod controller_collector_context;
pub mod global;
pub mod mutator_context;
pub mod plan_constraints;
pub mod tracelocal;
pub mod transitive_closure;
pub use self::global::AllocationSemantics;
pub use self::global::CopyContext;
pub use self::global::Plan;
pub use self::mutator_context::Mutator;
pub use self::mutator_context::MutatorContext;
pub use self::plan_constraints::PlanConstraints;
pub use self::tracelocal::TraceLocal;
pub use self::transitive_closure::TransitiveClosure;

pub mod gencopy;
pub mod nogc;
pub mod semispace;
