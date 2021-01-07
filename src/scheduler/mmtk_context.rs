use super::*;
use crate::util::OpaquePointer;
use crate::vm::{Collection, VMBinding};
use crate::{CopyContext, Plan, SelectedPlan, MMTK};
use crate::plan::global::PlanTypes;

trait GCWorkerLocal<VM: VMBinding> = WorkerLocal<MMTK<VM>>;

/// The global context for mmtk is `MMTK<VM>`.
impl<VM: VMBinding> Context for MMTK<VM> {
    type WorkerLocal = <SelectedPlan<VM> as PlanTypes>::CopyContext;
    fn spawn_worker(worker: &GCWorker<VM>, tls: OpaquePointer, _context: &'static Self) {
        VM::VMCollection::spawn_worker_thread(tls, Some(worker));
    }
}

/// Each GC should define their own Worker-local data in `CopyContext`.
impl<VM: VMBinding> WorkerLocal<MMTK<VM>> for <SelectedPlan<VM> as PlanTypes>::CopyContext {
    fn new(mmtk: &'static MMTK<VM>) -> Self {
        <<SelectedPlan<VM> as PlanTypes>::CopyContext as CopyContext>::new(mmtk)
    }
    fn init(&mut self, tls: OpaquePointer) {
        CopyContext::init(self, tls);
    }
}
