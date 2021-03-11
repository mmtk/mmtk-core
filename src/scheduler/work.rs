use super::worker::*;
use super::*;
use crate::mmtk::MMTK;
use crate::vm::VMBinding;
use std::any::{type_name, TypeId};

pub trait Work<C: Context>: 'static + Send + Sync {
    fn do_work(&mut self, worker: &mut Worker<C>, context: &'static C);
    #[inline]
    fn do_work_with_stat(&mut self, worker: &mut Worker<C>, context: &'static C) {
        let stat = worker
            .stat
            .measure_work(TypeId::of::<Self>(), type_name::<Self>());
        self.do_work(worker, context);
        stat.end_of_work(&mut worker.stat);
    }
}

/// A special kind of work that will execute on the coordinator (i.e. controller) thread
///
/// The coordinator thread holds the global monitor lock when executing `CoordinatorWork`s.
/// So, directly adding new work to any buckets will cause dead lock.
/// For this case, use `WorkBucket::add_with_priority_unsync` instead.
pub trait CoordinatorWork<C: Context>: 'static + Send + Sync + Work<C> {}

pub trait GCWork<VM: VMBinding>: 'static + Send + Sync + Sized + Work<MMTK<VM>> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>);
}

impl<VM: VMBinding, W: GCWork<VM>> Work<MMTK<VM>> for W {
    #[inline(always)]
    default fn do_work(&mut self, worker: &mut Worker<MMTK<VM>>, mmtk: &'static MMTK<VM>) {
        trace!("GCWork.do_work() {}", std::any::type_name::<W>());
        GCWork::do_work(self, worker, mmtk)
    }
}
