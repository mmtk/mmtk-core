//! Plan: ConcurrentSpan, a port of Go's garbage collector.
//!
//! The design is in `docs/design/go-gc-port.md`.  The plan is named after the space it is built
//! on (`SpanSpace`), the same way `ConcurrentImmix` is named after `ImmixSpace`.

pub(in crate::plan) mod gc_work;
pub(in crate::plan) mod global;
pub(in crate::plan) mod mutator;

pub use global::ConcurrentSpan;
