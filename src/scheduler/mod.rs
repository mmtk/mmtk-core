//! A general scheduler implementation. MMTk uses it to schedule GC-related work.

mod context;
pub(self) use context::Context;
pub(crate) use context::WorkerLocal;

mod mmtk_context;
pub(crate) use mmtk_context::GCWorkerLocal;
pub(crate) use mmtk_context::GCWorkerLocalPtr;

#[allow(clippy::module_inception)]
mod scheduler;
pub(crate) use scheduler::CoordinatorMessage;
pub(crate) use scheduler::MMTkScheduler;
pub(self) use scheduler::Scheduler;

mod stat;
mod work_counter;

mod work;
pub use work::CoordinatorWork;
pub use work::GCWork;
pub(crate) use work::Work;

<<<<<<< HEAD
pub use context::*;
pub use mmtk_context::*;
pub use scheduler::*;
pub use work::*;
pub use work_bucket::{WorkBucketStage, GCWorkBucket};
pub use worker::*;
=======
mod work_bucket;
pub use work_bucket::WorkBucketStage;
>>>>>>> 33fba23b... Reduce public visibility (#312)

mod worker;
pub use worker::GCWorker;
pub(crate) use worker::Worker;
pub(crate) use worker::WorkerLocalPtr;

pub(crate) mod gc_work;
pub use gc_work::ProcessEdgesWork;
// TODO: We shouldn't need to expose ScanStackRoot. However, OpenJDK uses it.
// We should do some refactoring related to Scanning::SCAN_MUTATORS_IN_SAFEPOINT
// to make sure this type is not exposed to the bindings.
pub use gc_work::ScanStackRoot;
