use crate::vm::{VMBinding, Collection};
use crate::{MMTK, Plan, SelectedPlan, CopyContext};
use super::*;
use crate::util::OpaquePointer;



pub trait Context: 'static + Send + Sync + Sized {
    type WorkerLocal: WorkerLocal<Self>;
    fn spawn_worker(worker: &'static Worker<Self>, _tls: OpaquePointer, context: &'static Self) {
        let worker_ptr = worker as *const Worker<Self> as usize;
        std::thread::spawn(move || {
            let worker = unsafe { &mut *(worker_ptr as *mut Worker<Self>) };
            worker.run(context);
        });
    }
}

impl Context for () {
    type WorkerLocal = ();
}

pub trait WorkerLocal<C: Context> {
    fn new(context: &'static C) -> Self;
}

impl <C: Context> WorkerLocal<C> for () {
    fn new(_: &'static C) -> Self { () }
}

trait GCWorkerLocal<VM: VMBinding> = WorkerLocal<MMTK<VM>>;

impl <VM: VMBinding> Context for MMTK<VM> {
    type WorkerLocal = <SelectedPlan::<VM> as Plan>::CopyContext;
    fn spawn_worker(worker: &GCWorker<VM>, tls: OpaquePointer, _context: &'static Self) {
        VM::VMCollection::spawn_worker_thread(tls, Some(worker));
    }
}

impl <VM: VMBinding> WorkerLocal<MMTK<VM>> for <SelectedPlan::<VM> as Plan>::CopyContext {
    fn new(mmtk: &'static MMTK<VM>) -> Self {
        <<SelectedPlan::<VM> as Plan>::CopyContext as CopyContext>::new(mmtk)
    }
}
