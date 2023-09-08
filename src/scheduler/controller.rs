//! The GC controller thread.
//!
//! MMTk has many GC threads.  There are many GC worker threads and one GC controller thread.
//! The GC controller thread responds to GC requests and coordinates the workers to perform GC.

use std::sync::Arc;

use crate::plan::gc_requester::GCRequester;
use crate::scheduler::gc_work::{EndOfGC, ScheduleCollection};
use crate::scheduler::{GCWork, WorkBucketStage};
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use crate::MMTK;

use super::{GCWorkScheduler, GCWorker};

/// The thread local struct for the GC controller, the counterpart of `GCWorker`.
pub struct GCController<VM: VMBinding> {
    /// The reference to the MMTk instance.
    mmtk: &'static MMTK<VM>,
    /// The reference to the GC requester.
    requester: Arc<GCRequester<VM>>,
    /// The reference to the scheduler.
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// The `GCWorker` is used to execute packets. The controller is also a `GCWorker`.
    coordinator_worker: GCWorker<VM>,
}

impl<VM: VMBinding> GCController<VM> {
    pub(crate) fn new(
        mmtk: &'static MMTK<VM>,
        requester: Arc<GCRequester<VM>>,
        scheduler: Arc<GCWorkScheduler<VM>>,
        coordinator_worker: GCWorker<VM>,
    ) -> Box<GCController<VM>> {
        Box::new(Self {
            mmtk,
            requester,
            scheduler,
            coordinator_worker,
        })
    }

    pub fn run(&mut self, tls: VMWorkerThread) {
        probe!(mmtk, gccontroller_run);
        // Initialize the GC worker for coordinator. We are not using the run() method from
        // GCWorker so we manually initialize the worker here.
        self.coordinator_worker.tls = tls;

        loop {
            debug!("[STWController: Waiting for request...]");
            self.requester.wait_for_request();
            debug!("[STWController: Request recieved.]");

            self.do_gc_until_completion_traced();
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

        // If all of the above failed, it means GC has finished.
        false
    }

    /// A wrapper method for [`do_gc_until_completion`](GCController::do_gc_until_completion) to insert USDT tracepoints.
    fn do_gc_until_completion_traced(&mut self) {
        probe!(mmtk, gc_start);
        self.do_gc_until_completion();
        probe!(mmtk, gc_end);
    }

    /// Coordinate workers to perform GC in response to a GC request.
    fn do_gc_until_completion(&mut self) {
        let gc_start = std::time::Instant::now();

        debug_assert!(
            self.scheduler.worker_monitor.debug_is_sleeping(),
            "Workers are still doing work when GC started."
        );

        // Add a ScheduleCollection work packet.  It is the seed of other work packets.
        self.scheduler.work_buckets[WorkBucketStage::Unconstrained].add(ScheduleCollection);

        // Notify only one worker at this time because there is only one work packet,
        // namely `ScheduleCollection`.
        self.scheduler.worker_monitor.resume_and_wait(false);

        // Gradually open more buckets as workers stop each time they drain all open bucket.
        loop {
            // Workers should only transition to the `Sleeping` state when all open buckets have
            // been drained.
            self.scheduler.assert_all_activated_buckets_are_empty();

            let new_work_available = self.find_more_work_for_workers();

            // GC finishes if there is no new work to do.
            if !new_work_available {
                break;
            }

            // Notify all workers because there should be many work packets available in the newly
            // opened bucket(s).
            self.scheduler.worker_monitor.resume_and_wait(true);
        }

        // All GC workers must have parked by now.
        debug_assert!(self.scheduler.worker_monitor.debug_is_sleeping());
        debug_assert!(!self.scheduler.worker_group.has_designated_work());
        debug_assert!(self.scheduler.all_buckets_empty());

        // Deactivate all work buckets to prepare for the next GC.
        // NOTE: There is no need to hold any lock.
        // Workers are in the `Sleeping` state.
        // so they will not wake up while we deactivate buckets.
        self.scheduler.deactivate_all();

        // Tell GC trigger that GC ended - this happens before EndOfGC where we resume mutators.
        self.mmtk.gc_trigger.policy.on_gc_end(self.mmtk);

        // Finalization: Resume mutators, reset gc states
        // Note: Resume-mutators must happen after all work buckets are closed.
        //       Otherwise, for generational GCs, workers will receive and process
        //       newly generated remembered-sets from those open buckets.
        //       But these remsets should be preserved until next GC.
        let mut end_of_gc = EndOfGC {
            elapsed: gc_start.elapsed(),
        };
        end_of_gc.do_work_with_stat(&mut self.coordinator_worker, self.mmtk);

        self.scheduler.debug_assert_all_buckets_deactivated();
    }
}
