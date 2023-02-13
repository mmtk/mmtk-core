use super::worker::*;
use crate::mmtk::MMTK;
use crate::vm::VMBinding;
#[cfg(feature = "work_packet_stats")]
use std::any::{type_name, TypeId};

/// A special kind of work that will execute on the coordinator (i.e. controller) thread
///
/// The coordinator thread holds the global monitor lock when executing `CoordinatorWork`s.
/// So, directly adding new work to any buckets will cause dead lock.
/// For this case, use `WorkBucket::add_with_priority_unsync` instead.
pub trait CoordinatorWork<VM: VMBinding>: 'static + Send + GCWork<VM> {}

pub trait GCWork<VM: VMBinding>: 'static + Send {
    /// Define the work for this packet. However, this is not supposed to be called directly.
    /// Usually `do_work_with_stat()` should be used.
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>);

    /// Do work and collect statistics. This internally calls `do_work()`. In most cases,
    /// this should be called rather than `do_work()` so that MMTk can correctly collect
    /// statistics for the work packets.
    /// If the feature "work_packet_stats" is not enabled, this call simply forwards the call
    /// to `do_work()`.
    fn do_work_with_stat(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        debug!("{}", std::any::type_name::<Self>());
        debug_assert!(!worker.tls.0.0.is_null(), "TLS must be set correctly for a GC worker before the worker does any work. GC Worker {} has no valid tls.", worker.ordinal);

        #[cfg(feature = "work_packet_stats")]
        // Start collecting statistics
        let stat = {
            let mut worker_stat = worker.shared.borrow_stat_mut();
            worker_stat.measure_work(TypeId::of::<Self>(), type_name::<Self>(), mmtk)
        };

        // Do the actual work
        self.do_work(worker, mmtk);

        #[cfg(feature = "work_packet_stats")]
        // Finish collecting statistics
        {
            let mut worker_stat = worker.shared.borrow_stat_mut();
            stat.end_of_work(&mut worker_stat);
        }
    }
}

use super::gc_work::ProcessEdgesWork;
use crate::plan::Plan;

/// This trait provides a group of associated types that are needed to
/// create GC work packets for a certain plan. For example, `GCWorkScheduler.schedule_common_work()`
/// needs this trait to schedule different work packets. For certain plans,
/// they may need to provide several types that implement this trait, e.g. one for
/// nursery GC, one for mature GC.
pub trait GCWorkContext {
    type VM: VMBinding;
    type PlanType: Plan<VM = Self::VM>;
    // We should use SFTProcessEdges as the default value for this associate type. However, this requires
    // `associated_type_defaults` which has not yet been stablized.
    type ProcessEdgesWorkType: ProcessEdgesWork<VM = Self::VM>;
}
