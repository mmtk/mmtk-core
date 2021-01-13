use super::*;
use crate::util::OpaquePointer;

/// The global context for the whole scheduling system.
/// This context is globally accessable for all work-packets, workers and the scheduler.
///
/// For mmtk, the global context is `MMTK<VM>`.
pub trait Context: 'static + Send + Sync + Sized {
    // type WorkerLocal: WorkerLocal<Self>;
    fn spawn_worker(worker: &'static Worker<Self>, _tls: OpaquePointer, context: &'static Self) {
        let worker_ptr = worker as *const Worker<Self> as usize;
        std::thread::spawn(move || {
            let worker = unsafe { &mut *(worker_ptr as *mut Worker<Self>) };
            worker.run(context);
        });
    }
}

/// A default implementation for scheduling systems that does not require a global context.
impl Context for () {
    // type WorkerLocal = ();
}

/// Thread-local data for each worker thread.
///
/// For mmtk, each gc can define their own worker-local data, to contain their required copy allocators and other stuffs.
pub trait WorkerLocal {
    // fn new(context: &'static C) -> Self;
    fn init(&mut self, _tls: OpaquePointer) {}
}

/// A default implementation for scheduling systems that does not require a worker-local context.
impl WorkerLocal for () {
    // fn new(_: &'static C) -> Self {}
}
