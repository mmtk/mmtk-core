//! A general scheduler implementation. MMTk uses it to schedule GC-related work.

/// This constant used to be the default capacity of work packets that process slots, such as
/// `TracingProcessSlots`.  But now it is an empirical value used by many work packets.
///
/// We expose this constant to the VM binding developers.  During root scanning, the VM binding
/// should call methods of [`RootsWorkFactory`] and pass lists of root slots or root nodes.  This
/// constant shall be used as the max lengths of those lists.
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

#[allow(unused)] // Used in doc comment.
use crate::vm::RootsWorkFactory;
