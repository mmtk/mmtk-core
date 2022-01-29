//! The GC controller thread.

use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::plan::controller_collector_context::ControllerCollectorContext;
use crate::scheduler::gc_work::ScheduleCollection;
use crate::scheduler::CoordinatorMessage;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use crate::MMTK;

use super::{GCWorkScheduler, GCWorker};

pub struct GCController<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
    requester: Arc<ControllerCollectorContext<VM>>,
    scheduler: Arc<GCWorkScheduler<VM>>,
    receiver: Receiver<CoordinatorMessage<VM>>,
    coordinator_worker: Box<GCWorker<VM>>,
}

impl<VM: VMBinding> GCController<VM> {
    pub fn new(
        mmtk: &'static MMTK<VM>,
        requester: Arc<ControllerCollectorContext<VM>>,
        scheduler: Arc<GCWorkScheduler<VM>>,
        receiver: Receiver<CoordinatorMessage<VM>>,
        coordinator_worker: Box<GCWorker<VM>>,
    ) -> Box<GCController<VM>> {
        Box::new(Self {
            mmtk,
            requester,
            scheduler,
            receiver,
            coordinator_worker,
        })
    }

    pub fn run(&mut self, tls: VMWorkerThread) {
        // Initialize the GC worker for coordinator. We are not using the run() method from
        // GCWorker so we manually initialize the worker here.
        self.coordinator_worker.tls = tls;

        loop {
            debug!("[STWController: Waiting for request...]");
            self.requester.wait_for_request();
            debug!("[STWController: Request recieved.]");

            // For heap growth logic
            // FIXME: This is not used. However, we probably want to set a 'user_triggered' flag
            // when GC is requested.
            // let user_triggered_collection: bool = SelectedPlan::is_user_triggered_collection();

            self.scheduler.set_initializer(Some(ScheduleCollection));
            self.wait_for_completion();
            debug!("[STWController: Worker threads complete!]");
        }
    }

    /// Drain the message queue and execute coordinator work.
    pub fn wait_for_completion(&mut self) {
        let worker = &mut self.coordinator_worker;
        let mmtk = self.mmtk;

        // At the start of a GC, we probably already have received a `ScheduleCollection` work. Run it now.
        if let Some(mut initializer) = self.scheduler.take_initializer() {
            initializer.do_work_with_stat(worker, mmtk);
        }
        loop {
            let message = self.receiver.recv().unwrap();
            match message {
                CoordinatorMessage::Work(mut work) => {
                    work.do_work_with_stat(worker, mmtk);
                }
                CoordinatorMessage::AllWorkerParked | CoordinatorMessage::BucketDrained => {
                    self.scheduler.update_buckets();
                }
            }
            let _guard = self.scheduler.worker_monitor.0.lock().unwrap();
            if self.scheduler.worker_group().all_parked() && self.scheduler.all_buckets_empty() {
                break;
            }
        }
        for message in self.receiver.try_iter() {
            if let CoordinatorMessage::Work(mut work) = message {
                work.do_work_with_stat(worker, mmtk);
                //self.process_coordinator_work(work);
            }
        }
        self.scheduler.deactivate_all();
        // Finalization: Resume mutators, reset gc states
        // Note: Resume-mutators must happen after all work buckets are closed.
        //       Otherwise, for generational GCs, workers will receive and process
        //       newly generated remembered-sets from those open buckets.
        //       But these remsets should be preserved until next GC.
        if let Some(mut finalizer) = self.scheduler.take_finalizer() {
            finalizer.do_work_with_stat(worker, mmtk);
        }
        self.scheduler.debug_assert_all_buckets_deactivated();
    }
}
