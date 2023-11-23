use super::worker::*;
use crate::mmtk::MMTK;
use crate::vm::VMBinding;
#[cfg(feature = "work_packet_stats")]
use std::any::{type_name, TypeId};

/// This defines a GC work packet which are assigned to the [`GCWorker`]s by the scheduler.
/// Work packets carry payloads that indicate the work to be done. For example, a work packet may
/// contain a pointer to a stack that must be scanned, or it may contain a large buffer of pointers
/// that need to be traced, or it might contain a range of static variables to be scanned, etc. The size
/// of the work packet will need to consider at least two points of tension: the work packet must be large
/// enough to ensure that the costs of managing the work packets do not dominate, and the packet must be
/// small enough that good load balancing is achieved.
pub trait GCWork<VM: VMBinding>: 'static + Send {
    /// Define the work for this packet. However, this is not supposed to be called directly.
    /// Usually `do_work_with_stat()` should be used.
    ///
    /// Most work packets are polled and executed in the worker's main loop ([`GCWorker::run`])
    /// using `do_work_with_stat`.  If `do_work` is called directly during the execution of another
    /// work packet, bypassing `do_work_with_stat()`, this work packet will not be counted into the
    /// number of work packets executed, and the execution time of this work packet will be counted
    /// as part of the execution time of the other work packet.  Only call this method directly if
    /// this is what you intend.  But you should always consider adding the work packet
    /// into a bucket so that other GC workers can execute it in parallel, unless the context-
    /// switching overhead is a problem.
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

    /// Get the compile-time static type name for the work packet.
    fn get_type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

use super::gc_work::ProcessEdgesWork;
use crate::plan::Plan;

/// This trait provides a group of associated types that are needed to
/// create GC work packets for a certain plan. For example, `GCWorkScheduler.schedule_common_work()`
/// needs this trait to schedule different work packets. For certain plans,
/// they may need to provide several types that implement this trait, e.g. one for
/// nursery GC, one for mature GC.
///
/// Note: Because `GCWorkContext` is often used as parameters of implementations of `GCWork`, we
/// let GCWorkContext require `Send + 'static`.  Since `GCWorkContext` is just a group of
/// associated types, its implementations should not have any actual fields other than
/// `PhantomData`, and will automatically have `Send + 'static`.
pub trait GCWorkContext: Send + 'static {
    type VM: VMBinding;
    type PlanType: Plan<VM = Self::VM>;
    // We should use SFTProcessEdges as the default value for this associate type. However, this requires
    // `associated_type_defaults` which has not yet been stablized.
    type ProcessEdgesWorkType: ProcessEdgesWork<VM = Self::VM>;
    type TPProcessEdges: ProcessEdgesWork<VM = Self::VM>;
}
