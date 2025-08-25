//! A general scheduler implementation. MMTk uses it to schedule GC-related work.

/// Buffer size for [`ProcessEdgesWork`] work packets. This constant is exposed to binding
/// developers so that they can use this value for places in their binding that interface with the
/// work packet system, specifically the transitive closure via `ProcessEdgesWork` work packets
/// such as roots gathering code or weak reference processing. In order to have better load
/// balancing, it is recommended that binding developers use this constant to split work up into
/// different work packets.
pub const EDGES_WORK_BUFFER_SIZE: usize = 4096;

pub(crate) mod affinity;

#[allow(clippy::module_inception)]
mod scheduler;
pub(crate) use scheduler::GCWorkScheduler;

mod stat;
mod work_counter;

pub(crate) mod work;
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
