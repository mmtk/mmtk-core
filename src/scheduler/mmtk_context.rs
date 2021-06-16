use super::*;
use crate::util::opaque_pointer::*;
use crate::vm::{Collection, VMBinding};
use crate::MMTK;

pub type GCWorkerLocalPtr = WorkerLocalPtr;
pub trait GCWorkerLocal = WorkerLocal;

/// The global context for mmtk is `MMTK<VM>`.
impl<VM: VMBinding> Context for MMTK<VM> {
    fn spawn_worker(worker: &GCWorker<VM>, tls: VMThread, _context: &'static Self) {
        VM::VMCollection::spawn_worker_thread(tls, Some(worker));
    }
}
