//! A general scheduler implementation. MMTk uses it to schedule GC-related work.

mod context;
pub use context::Context;
pub use context::WorkerLocal;

mod mmtk_context;
pub use mmtk_context::GCWorkerLocal;
pub use mmtk_context::GCWorkerLocalPtr;

#[allow(clippy::module_inception)]
mod scheduler;
pub use scheduler::MMTkScheduler;
pub use scheduler::CoordinatorMessage;
pub use scheduler::Scheduler;

mod stat;

mod work;
pub use work::Work;
pub use work::GCWork;
pub use work::CoordinatorWork;

mod work_bucket;
pub use work_bucket::WorkBucketStage;

mod worker;
pub use worker::Worker;
pub use worker::GCWorker;
pub use worker::WorkerLocalPtr;

// pub use context::*;
// pub use mmtk_context::*;
// pub use scheduler::*;
// pub use work::*;
// pub use worker::*;

pub mod gc_work;
pub use gc_work::ProcessEdgesWork;