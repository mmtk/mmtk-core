//! The GC controller thread.
//!
//! MMTk has many GC threads.  There are many GC worker threads and one GC controller thread.
//! The GC controller thread responds to GC requests and coordinates the workers to perform GC.

use std::sync::{Arc, Condvar, Mutex};

use crate::plan::gc_requester::GCRequester;
use crate::scheduler::gc_work::{EndOfGC, ScheduleCollection};
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use crate::MMTK;

use self::channel::{Event, Receiver};

use super::{CoordinatorWork, GCWorkScheduler, GCWorker};

pub(crate) mod channel;

/// The thread local struct for the GC controller, the counterpart of `GCWorker`.
pub struct GCController<VM: VMBinding> {
    /// The reference to the MMTk instance.
    mmtk: &'static MMTK<VM>,
    /// The reference to the GC requester.
    requester: Arc<GCRequester<VM>>,
    /// The reference to the scheduler.
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// Receive coordinator work packets and notifications from GC workers through this.
    receiver: Receiver<VM>,
    /// The `GCWorker` is used to execute packets. The controller is also a `GCWorker`.
    coordinator_worker: GCWorker<VM>,
}

impl<VM: VMBinding> GCController<VM> {
    pub(crate) fn new(
        mmtk: &'static MMTK<VM>,
        requester: Arc<GCRequester<VM>>,
        scheduler: Arc<GCWorkScheduler<VM>>,
        receiver: Receiver<VM>,
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

            self.do_gc_until_completion();
            debug!("[STWController: Worker threads complete!]");
        }
    }

    /// Find more work for workers to do.  Return true if more work is available.
    fn find_more_work_for_workers(&mut self) -> bool {
        if self.scheduler.worker_group.has_designated_work() {
            return true;
        }

        // See if any bucket has a sentinel.
        if self.scheduler.schedule_sentinels() {
            return true;
        }

        // Try to open new buckets.
        if self.scheduler.update_buckets() {
            return true;
        }

        // If all fo the above failed, it means GC has finished.
        false
    }

    /// Reset the "all workers parked" state and resume workers.
    fn reset_and_resume_workers(&mut self) {
        self.receiver.reset_all_workers_parked();
        self.scheduler.worker_monitor.notify_work_available(true);
        debug!("Workers resumed");
    }

    /// Handle the "all workers have parked" event.  Return true if GC is finished.
    fn on_all_workers_parked(&mut self) -> bool {
        assert!(self.scheduler.all_activated_buckets_are_empty());

        let new_work_available = self.find_more_work_for_workers();

        if new_work_available {
            self.reset_and_resume_workers();
            // If there is more work to do, GC has not finished.
            return false;
        }

        assert!(self.scheduler.all_buckets_empty());

        true
    }

    /// Process an event. Return true if the GC is finished.
    fn process_event(&mut self, message: Event<VM>) -> bool {
        match message {
            Event::Work(mut work) => {
                self.execute_coordinator_work(work.as_mut(), true);
                false
            }
            Event::AllParked => self.on_all_workers_parked(),
        }
    }

    /// Coordinate workers to perform GC in response to a GC request.
    pub fn do_gc_until_completion(&mut self) {
        let gc_start = std::time::Instant::now();
        // Schedule collection.
        self.execute_coordinator_work(&mut ScheduleCollection, true);

        // Tell GC trigger that GC started - this happens after ScheduleCollection so we
        // will know what kind of GC this is (e.g. nursery vs mature in gen copy, defrag vs fast in Immix)
        self.mmtk
            .plan
            .base()
            .gc_trigger
            .policy
            .on_gc_start(self.mmtk);

        // React to worker-generated events until finished.
        loop {
            let event = self.receiver.poll_event();
            let finished = self.process_event(event);
            if finished {
                break;
            }
        }

        // All GC workers must have parked by now.
        debug_assert!(self.scheduler.worker_monitor.debug_is_group_sleeping());
        debug_assert!(!self.scheduler.worker_group.has_designated_work());

        // Deactivate all work buckets to prepare for the next GC.
        // NOTE: There is no need to hold any lock.
        // All GC workers are doing "group sleeping" now,
        // so they will not wake up while we deactivate buckets.
        self.scheduler.deactivate_all();

        // Tell GC trigger that GC ended - this happens before EndOfGC where we resume mutators.
        self.mmtk.plan.base().gc_trigger.policy.on_gc_end(self.mmtk);

        // Finalization: Resume mutators, reset gc states
        // Note: Resume-mutators must happen after all work buckets are closed.
        //       Otherwise, for generational GCs, workers will receive and process
        //       newly generated remembered-sets from those open buckets.
        //       But these remsets should be preserved until next GC.
        let mut end_of_gc = EndOfGC {
            elapsed: gc_start.elapsed(),
        };

        self.execute_coordinator_work(&mut end_of_gc, false);

        self.scheduler.debug_assert_all_buckets_deactivated();
    }

    fn execute_coordinator_work(
        &mut self,
        work: &mut dyn CoordinatorWork<VM>,
        notify_workers: bool,
    ) {
        work.do_work_with_stat(&mut self.coordinator_worker, self.mmtk);

        if notify_workers {
            self.reset_and_resume_workers();
        };
    }
}
