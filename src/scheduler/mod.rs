//! A general scheduler implementation. MMTk uses it to schedule GC-related work.

#[allow(clippy::module_inception)]
mod scheduler;
pub(crate) use scheduler::CoordinatorMessage;
pub(crate) use scheduler::GCWorkScheduler;

mod stat;
pub(self) mod work_counter;

mod work;
pub use work::CoordinatorWork;
pub use work::GCWork;

mod work_bucket;
pub use work_bucket::WorkBucketStage;

mod worker;
pub use worker::GCWorker;
pub(crate) use worker::{GCWorkerLocal, GCWorkerLocalPtr};

pub(crate) mod gc_work;
pub use gc_work::ProcessEdgesWork;
// TODO: We shouldn't need to expose ScanStackRoot. However, OpenJDK uses it.
// We should do some refactoring related to Scanning::SCAN_MUTATORS_IN_SAFEPOINT
// to make sure this type is not exposed to the bindings.
pub use gc_work::ScanStackRoot;
