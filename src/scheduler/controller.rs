//! The GC controller thread.
//!
//! MMTk has many GC threads.  There are many GC worker threads and one GC controller thread.
//! The GC controller thread responds to GC requests and coordinates the workers to perform GC.

use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::plan::gc_requester::GCRequester;
use crate::scheduler::gc_work::{EndOfGC, ScheduleCollection};
use crate::scheduler::{CoordinatorMessage, GCWork};
use crate::util::VMWorkerThread;
use crate::vm::VMBinding;
use crate::MMTK;
use atomic::Ordering;

use super::{CoordinatorWork, GCWorkScheduler, GCWorker};

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

            self.do_gc_until_completion();
            debug!("[STWController: Worker threads complete!]");
        }
    }

    /// Process a message. Return true if the GC is finished.
    fn process_message(&mut self, message: CoordinatorMessage<VM>) -> bool {
        match message {
            CoordinatorMessage::Work(mut work) => {
                self.execute_coordinator_work(work.as_mut(), true);
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
        let gc_start = std::time::Instant::now();
        // Schedule collection.
        self.initiate_coordinator_work(&mut ScheduleCollection, true);

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
            // Note: GC workers may spuriously wake up, examining the states of work buckets and
            // trying to open them.  Use lock to ensure workers do not wake up when we deactivate
            // buckets.
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
        let mut end_of_gc = EndOfGC {
            elapsed: gc_start.elapsed(),
        };

        // Note: We cannot use `initiate_coordinator_work` here.  If we increment the
        // `pending_coordinator_packets` counter when a worker spuriously wakes up, it may try to
        // open new buckets and result in an assertion error.
        // See: https://github.com/mmtk/mmtk-core/issues/770
        //
        // The `pending_coordinator_packets` counter and the `initiate_coordinator_work` function
        // were introduced to prevent any buckets from being opened while `ScheduleCollection` or
        // `StopMutators` is being executed. (See the doc comment of `initiate_coordinator_work`.)
        // `EndOfGC` doesn't add any new work packets, therefore it does not need this layer of
        // synchronization.
        //
        // FIXME: We should redesign the synchronization mechanism to properly address the opening
        // condition of buckets.  See: https://github.com/mmtk/mmtk-core/issues/774
        end_of_gc.do_work_with_stat(&mut self.coordinator_worker, self.mmtk);

        self.scheduler.debug_assert_all_buckets_deactivated();
    }

    /// The controller uses this method to start executing a coordinator work immediately.
    ///
    /// Note: GC workers will start executing work packets as soon as individual work packets
    /// are added.  If the coordinator work (such as `ScheduleCollection`) adds multiple work
    /// packets into different buckets, workers may open subsequent buckets while the coordinator
    /// work still has packets to be added to prior buckets.  For this reason, we use the
    /// `pending_coordinator_packets` to prevent the workers from opening any work buckets while
    /// this coordinator work is being executed.
    ///
    /// # Arguments
    ///
    /// -   `work`: The work to execute.
    /// -   `notify_workers`: Notify one worker after the work is finished. Useful for proceeding
    ///     to the next work bucket stage.
    fn initiate_coordinator_work(
        &mut self,
        work: &mut dyn CoordinatorWork<VM>,
        notify_workers: bool,
    ) {
        self.scheduler
            .pending_coordinator_packets
            .fetch_add(1, Ordering::SeqCst);

        self.execute_coordinator_work(work, notify_workers)
    }

    fn execute_coordinator_work(
        &mut self,
        work: &mut dyn CoordinatorWork<VM>,
        notify_workers: bool,
    ) {
        work.do_work_with_stat(&mut self.coordinator_worker, self.mmtk);

        self.scheduler
            .pending_coordinator_packets
            .fetch_sub(1, Ordering::SeqCst);

        if notify_workers {
            // When a coordinator work finishes, there is a chance that all GC workers parked, and
            // no work packets are added to any open buckets.  We need to wake up one GC worker so
            // that it can open more work buckets.
            let _guard = self.scheduler.worker_monitor.0.lock().unwrap();
            self.scheduler.worker_monitor.1.notify_one();
        };
    }
}
