//! A general scheduler implementation. MMTk uses it to schedule GC-related work.

pub(crate) mod affinity;

#[allow(clippy::module_inception)]
mod scheduler;
pub(crate) use scheduler::GCWorkScheduler;

mod stat;
mod work_counter;

mod work;
pub use work::GCWork;
pub(crate) use work::GCWorkContext;

mod work_bucket;
pub use work_bucket::WorkBucketStage;

mod worker;
mod worker_goals;
mod worker_monitor;
pub(crate) use worker::current_worker_ordinal;
pub use worker::GCWorker;

pub(crate) mod gc_work;
pub use gc_work::ProcessEdgesWork;
