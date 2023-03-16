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

use self::monitor::Receiver;

use super::{CoordinatorWork, GCWorkScheduler, GCWorker};

pub(crate) mod monitor {
    use super::*;

    struct ControllerMonitorSync<VM: VMBinding> {
        coordinator_packets: Vec<Box<dyn CoordinatorWork<VM>>>,
        all_workers_parked: bool,
    }

    struct ControllerMonitor<VM: VMBinding> {
        sync: Mutex<ControllerMonitorSync<VM>>,
        cond: Condvar,
    }

    pub struct Sender<VM: VMBinding> {
        w2c: Arc<ControllerMonitor<VM>>,
    }

    impl<VM: VMBinding> Clone for Sender<VM> {
        fn clone(&self) -> Self {
            Self {
                w2c: self.w2c.clone(),
            }
        }
    }

    impl<VM: VMBinding> Sender<VM> {
        pub fn add_coordinator_work(&self, work: Box<dyn CoordinatorWork<VM>>) {
            let mut sync = self.w2c.sync.lock().unwrap();
            sync.coordinator_packets.push(work);
            debug!("Submitted coordinator work!");
            self.w2c.cond.notify_one();
        }

        pub fn notify_all_workers_parked(&self) {
            let mut sync = self.w2c.sync.lock().unwrap();
            sync.all_workers_parked = true;
            debug!("Notified all workers parked!");
            self.w2c.cond.notify_one();
        }
    }

    pub struct Receiver<VM: VMBinding> {
        w2c: Arc<ControllerMonitor<VM>>,
    }

    impl<VM: VMBinding> Receiver<VM> {
        pub(super) fn poll_event(&self) -> WorkerToControllerEvent<VM> {
            let mut sync = self.w2c.sync.lock().unwrap();
            loop {
                // Make sure the coordinator always sees packets before seeing "all parked".
                if let Some(work) = sync.coordinator_packets.pop() {
                    debug!("Received coordinator packet.");
                    return WorkerToControllerEvent::Work(work);
                }

                if sync.all_workers_parked {
                    debug!("Observed all workers parked.");
                    return WorkerToControllerEvent::AllParked;
                }

                sync = self.w2c.cond.wait(sync).unwrap();
            }
        }

        pub fn reset_all_workers_parked(&self) {
            let mut sync = self.w2c.sync.lock().unwrap();
            sync.all_workers_parked = false;
            debug!("All-workers-parked state reset.");
        }
    }

    pub(crate) fn make_channel<VM: VMBinding>() -> (Sender<VM>, Receiver<VM>) {
        let w2c = Arc::new(ControllerMonitor {
            sync: Mutex::new(ControllerMonitorSync {
                coordinator_packets: Default::default(),
                all_workers_parked: false,
            }),
            cond: Default::default(),
        });

        let worker_end = Sender { w2c: w2c.clone() };
        let controller_end = Receiver { w2c };
        (worker_end, controller_end)
    }
}

enum WorkerToControllerEvent<VM: VMBinding> {
    /// Send a work-packet to the coordinator thread/
    Work(Box<dyn CoordinatorWork<VM>>),
    /// Notify the coordinator thread that all GC tasks are finished.
    /// When sending this message, all the work buckets should be
    /// empty, and all the workers should be parked.
    AllParked,
}

/// The thread local struct for the GC controller, the counterpart of `GCWorker`.
pub struct GCController<VM: VMBinding> {
    /// The reference to the MMTk instance.
    mmtk: &'static MMTK<VM>,
    /// The reference to the GC requester.
    requester: Arc<GCRequester<VM>>,
    /// The reference to the scheduler.
    scheduler: Arc<GCWorkScheduler<VM>>,
    /// Receive messages from GC workers through this.
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

        false
    }

    fn resume_workers(&mut self) {
        self.receiver.reset_all_workers_parked();
        self.scheduler.worker_monitor.notify_work_available(true);
        debug!("Workers resumed");
    }

    fn on_all_parked(&mut self) -> bool {
        let new_work_available = self.find_more_work_for_workers();

        if new_work_available {
            self.resume_workers();
            // If there is more work to do, GC has not finished.
            return false;
        }

        assert!(self.scheduler.all_buckets_empty());

        true
    }

    /// Process an event. Return true if the GC is finished.
    fn process_event(&mut self, message: WorkerToControllerEvent<VM>) -> bool {
        match message {
            WorkerToControllerEvent::Work(mut work) => {
                self.execute_coordinator_work(work.as_mut(), true);
                false
            }
            WorkerToControllerEvent::AllParked => self.on_all_parked(),
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
        debug_assert!(self.scheduler.worker_monitor.is_group_sleeping());
        debug_assert!(!self.scheduler.worker_group.has_designated_work());

        // All GC workers are doing "group sleeping" now,
        // so they will not wake up when we deactivate buckets.
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
            self.resume_workers();
        };
    }
}
