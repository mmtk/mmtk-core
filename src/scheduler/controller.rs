//! The GC controller thread.
//!
//! MMTk has many GC threads.  There are many GC worker threads and one GC controller thread.
//! The GC controller thread responds to GC requests and coordinates the workers to perform GC.

use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::plan::gc_requester::GCRequester;
use crate::scheduler::gc_work::{EndOfGC, ScheduleCollection};
use crate::scheduler::CoordinatorMessage;
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use crate::MMTK;
use atomic::Ordering;

use super::{GCWork, GCWorkScheduler, GCWorker};

/// The thread local struct for the GC controller, the counterpart of `GCWorker`.
pub struct GCController<VM: VMBinding> {
    /// The reference to the MMTk instance.
    mmtk: &'static MMTK<VM>,
    /// The reference to the GC requester.
    requester: Arc<GCRequester<VM>>,
    /// The reference to the scheduler.
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// The receiving end of the channel to get controller/coordinator message from workers.
    receiver: Receiver<CoordinatorMessage<VM>>,
    /// The `GCWorker` is used to execute packets. The controller is also a `GCWorker`.
    coordinator_worker: GCWorker<VM>,
}

impl<VM: VMBinding> GCController<VM> {
    pub fn new(
        mmtk: &'static MMTK<VM>,
        requester: Arc<GCRequester<VM>>,
        scheduler: Arc<GCWorkScheduler<VM>>,
        receiver: Receiver<CoordinatorMessage<VM>>,
        coordinator_worker: GCWorker<VM>,
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

            self.do_gc_until_completion();
            debug!("[STWController: Worker threads complete!]");
        }
    }

    /// Process a message. Return true if the GC is finished.
    fn process_message(&mut self, message: CoordinatorMessage<VM>) -> bool {
        let worker = &mut self.coordinator_worker;
        let mmtk = self.mmtk;
        match message {
            CoordinatorMessage::Work(mut work) => {
                work.do_work_with_stat(worker, mmtk);
                let old_count = self
                    .scheduler
                    .pending_coordinator_packets
                    .fetch_sub(1, Ordering::SeqCst);
                if old_count == 1 {
                    // When the coordinator finishes executing all coordinator work packets,
                    // it is a chance to open more work buckets.
                    // Notify one worker so it can open buckets.
                    let _guard = self.scheduler.worker_monitor.0.lock().unwrap();
                    self.scheduler.worker_monitor.1.notify_one();
                }
                false
            }
            CoordinatorMessage::Finish => {
                // Quit only if all the buckets are empty.
                // For concurrent GCs, the coordinator thread may receive this message when
                // some buckets are still not empty. Under such case, the coordinator
                // should ignore the message.
                let _guard = self.scheduler.worker_monitor.0.lock().unwrap();
                self.scheduler.worker_group.all_parked() && self.scheduler.all_buckets_empty()
            }
        }
    }

    /// Coordinate workers to perform GC in response to a GC request.
    pub fn do_gc_until_completion(&mut self) {
        // Schedule collection.
        ScheduleCollection.do_work_with_stat(&mut self.coordinator_worker, self.mmtk);

        // Tell GC trigger that GC started - this happens after ScheduleCollection so we
        // will know what kind of GC this is (e.g. nursery vs mature in gen copy, defrag vs fast in Immix)
        self.mmtk
            .plan
            .base()
            .gc_trigger
            .policy
            .on_gc_start(self.mmtk);

        // Drain the message queue and execute coordinator work.
        loop {
            let message = self.receiver.recv().unwrap();
            let finished = self.process_message(message);
            if finished {
                break;
            }
        }
        debug_assert!(!self.scheduler.worker_group.has_designated_work());
        // Sometimes multiple finish messages will be sent. Skip them.
        for message in self.receiver.try_iter() {
            match message {
                CoordinatorMessage::Work(_) => unreachable!(),
                CoordinatorMessage::Finish => {}
            }
        }

        {
            let _guard = self.scheduler.worker_monitor.0.lock().unwrap();
            self.scheduler.deactivate_all();
        }

        // Tell GC trigger that GC ended - this happens before EndOfGC where we resume mutators.
        self.mmtk.plan.base().gc_trigger.policy.on_gc_end(self.mmtk);

        // Finalization: Resume mutators, reset gc states
        // Note: Resume-mutators must happen after all work buckets are closed.
        //       Otherwise, for generational GCs, workers will receive and process
        //       newly generated remembered-sets from those open buckets.
        //       But these remsets should be preserved until next GC.
        EndOfGC.do_work_with_stat(&mut self.coordinator_worker, self.mmtk);

        self.scheduler.debug_assert_all_buckets_deactivated();
    }
}
