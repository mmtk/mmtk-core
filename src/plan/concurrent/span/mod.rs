//! Plan: ConcurrentSpan, a port of Go's garbage collector.
//!
//! The design is in `docs/design/go-gc-port.md`.  The plan is named after the space it is built
//! on (`SpanSpace`), the same way `ConcurrentImmix` is named after `ImmixSpace`.

mod assist;
// Some of the CPU limiter's inputs (idle time) are not wired in.
#[allow(dead_code)]
pub(crate) mod cpu_limiter;
pub(in crate::plan) mod gc_work;
pub(crate) mod global;
pub(crate) mod greentea;
pub(in crate::plan) mod mutator;
// Some of the pacer's inputs (assists, scan work, worker time) are not wired in yet.
#[allow(dead_code)]
pub(crate) mod pacer;
mod pacing;
mod stats;

pub use global::ConcurrentSpan;
