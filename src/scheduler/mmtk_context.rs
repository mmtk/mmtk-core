use super::*;
use crate::util::OpaquePointer;
use crate::vm::{Collection, VMBinding};
use crate::MMTK;

pub type GCWorkerLocalPtr = WorkerLocalPtr;
pub trait GCWorkerLocal = WorkerLocal;

/// The global context for mmtk is `MMTK<VM>`.
impl<VM: VMBinding> Context for MMTK<VM> {
    fn spawn_worker(worker: &GCWorker<VM>, tls: OpaquePointer, _context: &'static Self) {
        VM::VMCollection::spawn_worker_thread(tls, Some(worker));
    }
}
