//! The GC controller thread.

use std::sync::Arc;

use crate::plan::controller_collector_context::ControllerCollectorContext;
use crate::scheduler::gc_work::ScheduleCollection;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;

use super::{GCWorkScheduler, GCWorker};

pub struct GCController<VM: VMBinding> {
    requester: Arc<ControllerCollectorContext<VM>>,
    scheduler: Arc<GCWorkScheduler<VM>>,
    coordinator_worker: Box<GCWorker<VM>>,
}

impl<VM: VMBinding> GCController<VM> {
    pub fn new(
        requester: Arc<ControllerCollectorContext<VM>>,
        scheduler: Arc<GCWorkScheduler<VM>>,
        coordinator_worker: Box<GCWorker<VM>>,
    ) -> Box<GCController<VM>> {
        Box::new(Self {
            requester,
            scheduler,
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
            self.scheduler
                .wait_for_completion(&mut self.coordinator_worker);
            debug!("[STWController: Worker threads complete!]");
        }
    }
}
