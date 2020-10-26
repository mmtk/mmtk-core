use super::stat::SchedulerStat;
use super::work::Work;
use super::work_bucket::*;
use super::worker::{Worker, WorkerGroup};
use super::*;
use crate::mmtk::MMTK;
use crate::plan::Plan;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

pub enum CoordinatorMessage<C: Context> {
    Work(Box<dyn CoordinatorWork<C>>),
    AllWorkerParked,
    BucketDrained,
}

pub struct Scheduler<C: Context> {
    /// Works that are scheduable at any time
    pub unconstrained_works: WorkBucket<C>,
    /// Works that are scheduable within Stop-the-world
    pub prepare_stage: WorkBucket<C>,
    pub closure_stage: WorkBucket<C>,
    pub release_stage: WorkBucket<C>,
    pub final_stage: WorkBucket<C>,
    /// Works for the coordinator thread
    pub coordinator_works: WorkBucket<C>,
    /// workers
    worker_group: Option<Arc<WorkerGroup<C>>>,
    /// Condition Variable for worker synchronization
    pub worker_monitor: Arc<(Mutex<()>, Condvar)>,
    context: Option<&'static C>,
    coordinator_worker: Option<Worker<C>>,
    /// A message channel to send new coordinator works and other actions to the coordinator thread
    pub channel: (
        Sender<CoordinatorMessage<C>>,
        Receiver<CoordinatorMessage<C>>,
    ),
    startup: Mutex<Option<Box<dyn CoordinatorWork<C>>>>,
    finalizer: Mutex<Option<Box<dyn CoordinatorWork<C>>>>,
}

unsafe impl<C: Context> Send for Scheduler<C> {}
unsafe impl<C: Context> Sync for Scheduler<C> {}

impl<C: Context> Scheduler<C> {
    pub fn new() -> Arc<Self> {
        let worker_monitor: Arc<(Mutex<()>, Condvar)> = Default::default();
        Arc::new(Self {
            unconstrained_works: WorkBucket::new(true, worker_monitor.clone()), // `default_bucket` is always activated
            prepare_stage: WorkBucket::new(false, worker_monitor.clone()),
            closure_stage: WorkBucket::new(false, worker_monitor.clone()),
            release_stage: WorkBucket::new(false, worker_monitor.clone()),
            final_stage: WorkBucket::new(false, worker_monitor.clone()),
            coordinator_works: WorkBucket::new(true, worker_monitor.clone()),
            worker_group: None,
            worker_monitor,
            context: None,
            coordinator_worker: None,
            channel: channel(),
            startup: Mutex::new(None),
            finalizer: Mutex::new(None),
        })
    }

    pub fn initialize(
        self: &'static Arc<Self>,
        num_workers: usize,
        context: &'static C,
        tls: OpaquePointer,
    ) {
        let mut self_mut = self.clone();
        let self_mut = unsafe { Arc::get_mut_unchecked(&mut self_mut) };

        self_mut.context = Some(context);
        self_mut.coordinator_worker = Some(Worker::new(0, None, Arc::downgrade(&self)));
        self_mut.worker_group = Some(WorkerGroup::new(num_workers, Arc::downgrade(&self)));
        self.worker_group
            .as_ref()
            .unwrap()
            .spawn_workers(tls, context);

        self_mut.closure_stage.set_open_condition(move || {
            self.unconstrained_works.is_drained()
                && self.prepare_stage.is_drained()
                && self.worker_group().all_parked()
        });
        self_mut.release_stage.set_open_condition(move || {
            self.unconstrained_works.is_drained()
                && self.prepare_stage.is_drained()
                && self.closure_stage.is_drained()
                && self.worker_group().all_parked()
        });
        self_mut.final_stage.set_open_condition(move || {
            self.unconstrained_works.is_drained()
                && self.prepare_stage.is_drained()
                && self.closure_stage.is_drained()
                && self.release_stage.is_drained()
                && self.worker_group().all_parked()
        });
    }

    pub fn initialize_worker(self: &Arc<Self>, tls: OpaquePointer) {
        let mut self_mut = self.clone();
        let self_mut = unsafe { Arc::get_mut_unchecked(&mut self_mut) };
        self_mut.coordinator_worker.as_mut().unwrap().init(tls);
    }

    pub fn set_initializer<W: CoordinatorWork<C>>(&self, w: Option<W>) {
        *self.startup.lock().unwrap() = w.map(|w| box w as Box<dyn CoordinatorWork<C>>);
    }

    pub fn set_finalizer<W: CoordinatorWork<C>>(&self, w: Option<W>) {
        *self.finalizer.lock().unwrap() = w.map(|w| box w as Box<dyn CoordinatorWork<C>>);
    }

    pub fn worker_group(&self) -> Arc<WorkerGroup<C>> {
        self.worker_group.as_ref().unwrap().clone()
    }

    fn all_buckets_empty(&self) -> bool {
        self.unconstrained_works.is_empty()
            && self.prepare_stage.is_empty()
            && self.closure_stage.is_empty()
            && self.release_stage.is_empty()
            && self.final_stage.is_empty()
    }

    /// Open buckets if their conditions are met
    fn update_buckets(&self) {
        let mut buckets_updated = false;
        buckets_updated |= self.prepare_stage.update();
        buckets_updated |= self.closure_stage.update();
        buckets_updated |= self.release_stage.update();
        buckets_updated |= self.final_stage.update();
        if buckets_updated {
            // Notify the workers for new works
            let _guard = self.worker_monitor.0.lock().unwrap();
            self.worker_monitor.1.notify_all();
        }
    }

    /// Execute coordinator works, in the controller thread
    fn process_coordinator_work(&self, mut work: Box<dyn CoordinatorWork<C>>) {
        let worker_ptr = self.coordinator_worker.as_ref().unwrap() as *const _ as *mut Worker<C>;
        let context = self.context.unwrap();
        let worker = unsafe { &mut *worker_ptr };
        work.do_work_with_stat(worker, context);
    }

    /// Drain the message queue and execute coordinator works
    pub fn wait_for_completion(&self) {
        // At the start of a GC, we probably already have received a `ScheduleCollection` work. Run it now.
        if let Some(initializer) = self.startup.lock().unwrap().take() {
            self.process_coordinator_work(initializer);
        }
        loop {
            let message = self.channel.1.recv().unwrap();
            match message {
                CoordinatorMessage::Work(work) => {
                    self.process_coordinator_work(work);
                }
                CoordinatorMessage::AllWorkerParked | CoordinatorMessage::BucketDrained => {
                    self.update_buckets();
                }
            }
            let _guard = self.worker_monitor.0.lock().unwrap();
            if self.worker_group().all_parked() && self.all_buckets_empty() {
                break;
            }
        }
        for message in self.channel.1.try_iter() {
            if let CoordinatorMessage::Work(work) = message {
                self.process_coordinator_work(work);
            }
        }
        self.deactivate_all();
        // Finalization: Resume mutators, reset gc states
        // Note: Resume-mutators must happen after all work buckets are closed.
        //       Otherwise, for generational GCs, workers will receive and process
        //       newly generated remembered-sets from those open buckets.
        //       But these remsets should be preserved until next GC.
        if let Some(finalizer) = self.finalizer.lock().unwrap().take() {
            self.process_coordinator_work(finalizer);
        }
        debug_assert!(!self.prepare_stage.is_activated());
        debug_assert!(!self.closure_stage.is_activated());
        debug_assert!(!self.final_stage.is_activated());
        debug_assert!(!self.release_stage.is_activated());
    }

    pub fn deactivate_all(&self) {
        self.prepare_stage.deactivate();
        self.closure_stage.deactivate();
        self.release_stage.deactivate();
        self.final_stage.deactivate();
    }

    pub fn reset_state(&self) {
        // self.prepare_stage.deactivate();
        self.closure_stage.deactivate();
        self.release_stage.deactivate();
        self.final_stage.deactivate();
    }

    pub fn add_coordinator_work(&self, work: impl CoordinatorWork<C>, worker: &Worker<C>) {
        worker
            .sender
            .send(CoordinatorMessage::Work(box work))
            .unwrap();
    }

    #[inline]
    fn pop_scheduable_work(&self, worker: &Worker<C>) -> Option<(Box<dyn Work<C>>, bool)> {
        if let Some(work) = worker.local_works.poll() {
            return Some((work, worker.local_works.is_empty()));
        }
        if let Some(work) = self.unconstrained_works.poll() {
            return Some((work, self.unconstrained_works.is_empty()));
        }
        if let Some(work) = self.prepare_stage.poll() {
            return Some((work, self.prepare_stage.is_empty()));
        }
        if let Some(work) = self.closure_stage.poll() {
            return Some((work, self.closure_stage.is_empty()));
        }
        if let Some(work) = self.release_stage.poll() {
            return Some((work, self.release_stage.is_empty()));
        }
        if let Some(work) = self.final_stage.poll() {
            return Some((work, self.final_stage.is_empty()));
        }
        None
    }

    /// Get a scheduable work. Called by workers
    #[inline]
    pub fn poll(&self, worker: &Worker<C>) -> Box<dyn Work<C>> {
        let work = if let Some((work, bucket_is_empty)) = self.pop_scheduable_work(worker) {
            if bucket_is_empty {
                worker
                    .sender
                    .send(CoordinatorMessage::BucketDrained)
                    .unwrap();
            }
            work
        } else {
            self.poll_slow(worker)
        };
        work
    }

    #[cold]
    fn poll_slow(&self, worker: &Worker<C>) -> Box<dyn Work<C>> {
        debug_assert!(!worker.is_parked());
        let mut guard = self.worker_monitor.0.lock().unwrap();
        loop {
            debug_assert!(!worker.is_parked());
            if let Some((work, bucket_is_empty)) = self.pop_scheduable_work(worker) {
                if bucket_is_empty {
                    worker
                        .sender
                        .send(CoordinatorMessage::BucketDrained)
                        .unwrap();
                }
                return work;
            }
            // Park this worker
            worker.parked.store(true, Ordering::SeqCst);
            if worker.group().unwrap().all_parked() {
                worker
                    .sender
                    .send(CoordinatorMessage::AllWorkerParked)
                    .unwrap();
            }
            // Wait
            guard = self.worker_monitor.1.wait(guard).unwrap();
            // Unpark this worker
            worker.parked.store(false, Ordering::SeqCst);
        }
    }

    pub fn enable_stat(&self) {
        for worker in &self.worker_group().workers {
            worker.stat.enable();
        }
        self.coordinator_worker.as_ref().unwrap().stat.enable();
    }

    pub fn statistics(&self) -> HashMap<String, String> {
        let mut summary = SchedulerStat::default();
        for worker in &self.worker_group().workers {
            summary.merge(&worker.stat);
        }
        summary.merge(&self.coordinator_worker.as_ref().unwrap().stat);
        summary.harness_stat()
    }
}

pub type MMTkScheduler<VM> = Scheduler<MMTK<VM>>;

impl<VM: VMBinding> MMTkScheduler<VM> {
    pub fn notify_mutators_paused(&self, mmtk: &'static MMTK<VM>) {
        mmtk.plan.base().control_collector_context.clear_request();
        debug_assert!(!self.prepare_stage.is_activated());
        self.prepare_stage.activate();
        let _guard = self.worker_monitor.0.lock().unwrap();
        self.worker_monitor.1.notify_all();
    }
}
