use super::worker::*;
use crate::vm::VMBinding;
use crate::mmtk::MMTK;
use super::*;
use std::any::{TypeId, type_name};



pub trait Work<C: Context>: 'static + Send + Sync {
    fn do_work(&mut self, worker: &mut Worker<C>, context: &'static C);
    fn do_work_with_stat(&mut self, worker: &mut Worker<C>, context: &'static C) {
        let stat = worker.stat.measure_work(TypeId::of::<Self>(), type_name::<Self>());
        self.do_work(worker, context);
        stat.end_of_work(&mut worker.stat);
    }
}

impl <VM: VMBinding> PartialEq for Box<dyn Work<VM>> {
    /// Address comparison. Two different work instances cannot have the same address
    #[allow(clippy::vtable_address_comparisons)]
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() as *const dyn Work<VM> == other.as_ref() as *const dyn Work<VM>
    }
}

impl <VM: VMBinding> Eq for Box<dyn Work<VM>> {}

/// A special kind of work that will execute on the coorddinator (i.e. controller) thread
///
/// The coorddinator thread holds the global monitor lock when executing `CoordinatorWork`s.
/// So, directly adding new works to any buckets will cause dead lock.
/// For this case, use `WorkBucket::add_with_priority_unsync` instead.
pub trait CoordinatorWork<C: Context>: 'static + Send + Sync + Work<C> {}

pub trait GCWork<VM: VMBinding>: 'static + Send + Sync + Sized + Work<MMTK<VM>> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>);
}

impl <VM: VMBinding, W: GCWork<VM>> Work<MMTK<VM>> for W {
    #[inline(always)]
    default fn do_work(&mut self, worker: &mut Worker<MMTK<VM>>, mmtk: &'static MMTK<VM>) {
        GCWork::do_work(self, worker, mmtk)
    }
}
