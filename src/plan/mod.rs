pub mod plan;
pub mod transitive_closure;
pub mod controller_collector_context;
mod phase;
mod trace;
pub mod mutator_context;
pub mod collector_context;

pub use self::plan::Plan;
pub use self::transitive_closure::TransitiveClosure;
pub use self::phase::Phase;
pub use self::mutator_context::MutatorContext;

#[cfg(feature = "nogc")]
pub mod nogc;

#[cfg(feature = "nogc")]
pub use self::nogc as selected_plan;

#[cfg(feature = "semispace")]
pub mod semispace;

#[cfg(feature = "semispace")]
pub use self::semispace as selected_plan;