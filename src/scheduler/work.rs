use super::worker::*;
use crate::mmtk::MMTK;
use crate::vm::VMBinding;
use std::any::{type_name, TypeId};

/// A special kind of work that will execute on the coordinator (i.e. controller) thread
///
/// The coordinator thread holds the global monitor lock when executing `CoordinatorWork`s.
/// So, directly adding new work to any buckets will cause dead lock.
/// For this case, use `WorkBucket::add_with_priority_unsync` instead.
pub trait CoordinatorWork<VM: VMBinding>: 'static + Send + GCWork<VM> {}

pub trait GCWork<VM: VMBinding>: 'static + Send {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>);
    #[inline]
    fn do_work_with_stat(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        debug!("{}", std::any::type_name::<Self>());
        let stat = worker
            .stat
            .measure_work(TypeId::of::<Self>(), type_name::<Self>(), mmtk);
        self.do_work(worker, mmtk);
        stat.end_of_work(&mut worker.stat);
    }
}

use super::gc_work::ProcessEdgesWork;
use crate::plan::Plan;
use crate::policy::copy_context::CopyContext;

/// This trait provides a group of associated types that are needed to
/// create GC work packets for a certain plan. For example, `GCWorkScheduler.schedule_common_work()`
/// needs this trait to schedule different work packets. For certain plans,
/// they may need to provide several types that implement this trait, e.g. one for
/// nursery GC, one for mature GC.
pub trait GCWorkContext {
    type VM: VMBinding;
    type PlanType: Plan<VM = Self::VM>;
    type CopyContextType: CopyContext<VM = Self::VM> + GCWorkerLocal;
    type ProcessEdgesWorkType: ProcessEdgesWork<VM = Self::VM>;
}
