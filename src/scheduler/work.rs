use super::worker::*;
use crate::mmtk::MMTK;
use crate::vm::VMBinding;
#[cfg(feature = "work_packet_stats")]
use std::any::{type_name, TypeId};

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

    /// Get the compile-time static type name for the work packet.
    fn get_type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Get a human-readable size of the work packet if it makes sense.
    ///
    /// This method is useful for debug purposes, and is especially useful when visualising the
    /// work packets during a GC.
    ///
    /// The semantics is unspecified.  For work packets that contains a list of work items, such as
    /// object references or edges, the size should usually be the number of items.  If size does
    /// not make sense for the work packet, this method may return `None`.
    fn debug_get_size(&self) -> Option<usize> {
        None
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
