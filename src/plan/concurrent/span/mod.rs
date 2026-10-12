//! Plan: ConcurrentSpan, a port of Go's garbage collector.
//!
//! The design is in `docs/design/go-gc-port.md`.  The plan is named after the space it is built
//! on (`SpanSpace`), the same way `ConcurrentImmix` is named after `ImmixSpace`.

// The CPU limiter is not wired into the plan yet.
#[allow(dead_code)]
pub(crate) mod cpu_limiter;
pub(in crate::plan) mod gc_work;
pub(in crate::plan) mod global;
pub(in crate::plan) mod mutator;
// The pacer is not wired into a GC trigger yet.
#[allow(dead_code)]
pub(crate) mod pacer;

pub use global::ConcurrentSpan;
