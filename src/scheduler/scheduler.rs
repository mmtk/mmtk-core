use super::stat::SchedulerStat;
use super::work::Work;
use super::work_bucket::*;
use super::worker::{Worker, WorkerGroup};
use super::*;
use crate::mmtk::MMTK;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use enum_map::{enum_map, EnumMap};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, RwLock};

pub enum CoordinatorMessage<C: Context> {
    Work(Box<dyn CoordinatorWork<C>>),
    AllWorkerParked,
    BucketDrained,
}

pub struct Scheduler<C: Context> {
    pub work_buckets: EnumMap<WorkBucketStage, WorkBucket<C>>,
    /// Work for the coordinator thread
    pub coordinator_work: WorkBucket<C>,
    /// workers
    worker_group: Option<Arc<WorkerGroup<C>>>,
    /// Condition Variable for worker synchronization
    pub worker_monitor: Arc<(Mutex<()>, Condvar)>,
    context: Option<&'static C>,
    coordinator_worker: Option<RwLock<Worker<C>>>,
    /// A message channel to send new coordinator work and other actions to the coordinator thread
    channel: (
        Sender<CoordinatorMessage<C>>,
        Receiver<CoordinatorMessage<C>>,
    ),
    startup: Mutex<Option<Box<dyn CoordinatorWork<C>>>>,
    finalizer: Mutex<Option<Box<dyn CoordinatorWork<C>>>>,
    /// A callback to be fired after the `Closure` bucket is drained.
    /// This callback should return `true` if it adds more work packets to the
    /// `Closure` bucket. `WorkBucket::can_open` then consult this return value
    /// to prevent the GC from proceeding to the next stage, if we still have
    /// `Closure` work to do.
    ///
    /// We use this callback to process ephemeron objects. `closure_end` can re-enable
    /// the `Closure` bucket multiple times to iteratively discover and process
    /// more ephemeron objects.
    closure_end: Mutex<Option<Box<dyn Send + Fn() -> bool>>>,
}

// The 'channel' inside Scheduler disallows Sync for Scheduler. We have to make sure we use channel properly:
// 1. We should never directly use Sender. We clone the sender and let each worker have their own copy.
// 2. Only the coordinator can use Receiver.
// TODO: We should remove channel from Scheduler, and directly send Sender/Receiver when creating the coordinator and
// the workers.
unsafe impl<C: Context> Sync for Scheduler<C> {}

impl<C: Context> Scheduler<C> {
    pub fn new() -> Arc<Self> {
        let worker_monitor: Arc<(Mutex<()>, Condvar)> = Default::default();
        Arc::new(Self {
            work_buckets: enum_map! {
                WorkBucketStage::Unconstrained => WorkBucket::new(true, worker_monitor.clone()),
                WorkBucketStage::Prepare => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::ForwardRoots => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::Closure => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::RefClosure => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::RefForwarding => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::Release => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::Final => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::PreClosure => WorkBucket::new(false, worker_monitor.clone()),
                WorkBucketStage::PostClosure => WorkBucket::new(false, worker_monitor.clone()),
            },
            coordinator_work: WorkBucket::new(true, worker_monitor.clone()),
            worker_group: None,
            worker_monitor,
            context: None,
            coordinator_worker: None,
            channel: channel(),
            startup: Mutex::new(None),
            finalizer: Mutex::new(None),
            closure_end: Mutex::new(None),
        })
    }

    #[inline]
    pub fn num_workers(&self) -> usize {
        self.worker_group.as_ref().unwrap().worker_count()
    }

    pub fn initialize(
        self: &'static Arc<Self>,
        num_workers: usize,
        context: &'static C,
        tls: VMThread,
    ) {
        use crate::scheduler::work_bucket::WorkBucketStage::*;
        let num_workers = if cfg!(feature = "single_worker") {
            1
        } else {
            num_workers
        };

        let mut self_mut = self.clone();
        let self_mut = unsafe { Arc::get_mut_unchecked(&mut self_mut) };

        self_mut.context = Some(context);
        self_mut.coordinator_worker = Some(RwLock::new(Worker::new(
            233,
            Arc::downgrade(&self),
            true,
            self.channel.0.clone(),
        )));
        self_mut.worker_group = Some(WorkerGroup::new(
            num_workers,
            Arc::downgrade(&self),
            self.channel.0.clone(),
        ));
        self.worker_group
            .as_ref()
            .unwrap()
            .spawn_workers(tls, context);

        {
            // Unconstrained is always open. Prepare will be opened at the beginning of a GC.
            // This vec will grow for each stage we call with open_next()
            let mut open_stages: Vec<WorkBucketStage> = vec![Unconstrained, Prepare];
            // The rest will open after the previous stage is done.
            let mut open_next = |s: WorkBucketStage| {
                let cur_stages = open_stages.clone();
                self_mut.work_buckets[s].set_open_condition(move || {
                    let should_open =
                        self.are_buckets_drained(&cur_stages) && self.worker_group().all_parked();
                    // Additional check before the `RefClosure` bucket opens.
                    if should_open && s == WorkBucketStage::RefClosure {
                        if let Some(closure_end) = self.closure_end.lock().unwrap().as_ref() {
                            if closure_end() {
                                // Don't open `RefClosure` if `closure_end` added more works to `Closure`.
                                return false;
                            }
                        }
                    }
                    should_open
                });
                open_stages.push(s);
            };

            open_next(ForwardRoots);
            open_next(PreClosure);
            open_next(Closure);
            open_next(PostClosure);
            open_next(RefClosure);
            open_next(RefForwarding);
            open_next(Release);
            open_next(Final);
        }
    }

    fn are_buckets_drained(&self, buckets: &[WorkBucketStage]) -> bool {
        buckets.iter().all(|&b| self.work_buckets[b].is_drained())
    }

    pub fn initialize_worker(self: &Arc<Self>, tls: VMWorkerThread) {
        let mut coordinator_worker = self.coordinator_worker.as_ref().unwrap().write().unwrap();
        coordinator_worker.init(tls);
    }

    pub fn set_initializer<W: CoordinatorWork<C>>(&self, w: Option<W>) {
        *self.startup.lock().unwrap() = w.map(|w| box w as Box<dyn CoordinatorWork<C>>);
    }

    pub fn set_finalizer<W: CoordinatorWork<C>>(&self, w: Option<W>) {
        *self.finalizer.lock().unwrap() = w.map(|w| box w as Box<dyn CoordinatorWork<C>>);
    }

    pub fn on_closure_end(&self, f: Box<dyn Send + Fn() -> bool>) {
        *self.closure_end.lock().unwrap() = Some(f);
    }

    pub fn worker_group(&self) -> Arc<WorkerGroup<C>> {
        self.worker_group.as_ref().unwrap().clone()
    }

    fn all_buckets_empty(&self) -> bool {
        self.work_buckets.values().all(|bucket| bucket.is_empty())
    }

    /// Open buckets if their conditions are met
    fn update_buckets(&self) {
        let mut buckets_updated = false;
        for (id, bucket) in self.work_buckets.iter() {
            if id == WorkBucketStage::Unconstrained {
                continue;
            }
            let x = bucket.update();
            if x {
                println!("Activate WorkBucketStage::{:?}", id);
            }
            buckets_updated |= x;
        }
        if buckets_updated {
            // Notify the workers for new work
            let _guard = self.worker_monitor.0.lock().unwrap();
            self.worker_monitor.1.notify_all();
        }
    }

    /// Execute coordinator work, in the controller thread
    fn process_coordinator_work(&self, mut work: Box<dyn CoordinatorWork<C>>) {
        let mut coordinator_worker = self.coordinator_worker.as_ref().unwrap().write().unwrap();
        let context = self.context.unwrap();
        work.do_work_with_stat(&mut coordinator_worker, context);
    }

    /// Drain the message queue and execute coordinator work. Only the coordinator should call this.
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
        println!("Deactivate All Work Buckets");
        self.deactivate_all();
        // Finalization: Resume mutators, reset gc states
        // Note: Resume-mutators must happen after all work buckets are closed.
        //       Otherwise, for generational GCs, workers will receive and process
        //       newly generated remembered-sets from those open buckets.
        //       But these remsets should be preserved until next GC.
        if let Some(finalizer) = self.finalizer.lock().unwrap().take() {
            self.process_coordinator_work(finalizer);
        }
        debug_assert!(!self.work_buckets[WorkBucketStage::Prepare].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::ForwardRoots].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::PreClosure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Closure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::PostClosure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::RefClosure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::RefForwarding].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Release].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Final].is_activated());
    }

    pub fn assert_all_deactivated(&self) {
        debug_assert!(!self.work_buckets[WorkBucketStage::Prepare].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::ForwardRoots].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::PreClosure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Closure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::PostClosure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::RefClosure].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::RefForwarding].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Release].is_activated());
        debug_assert!(!self.work_buckets[WorkBucketStage::Final].is_activated());
    }

    pub fn deactivate_all(&self) {
        self.work_buckets[WorkBucketStage::Prepare].deactivate();
        self.work_buckets[WorkBucketStage::ForwardRoots].deactivate();
        self.work_buckets[WorkBucketStage::PreClosure].deactivate();
        self.work_buckets[WorkBucketStage::Closure].deactivate();
        self.work_buckets[WorkBucketStage::PostClosure].deactivate();
        self.work_buckets[WorkBucketStage::RefClosure].deactivate();
        self.work_buckets[WorkBucketStage::RefForwarding].deactivate();
        self.work_buckets[WorkBucketStage::Release].deactivate();
        self.work_buckets[WorkBucketStage::Final].deactivate();
    }

    pub fn reset_state(&self) {
        // self.work_buckets[WorkBucketStage::Prepare].deactivate();
        self.work_buckets[WorkBucketStage::ForwardRoots].deactivate();
        self.work_buckets[WorkBucketStage::PreClosure].deactivate();
        self.work_buckets[WorkBucketStage::PostClosure].deactivate();
        self.work_buckets[WorkBucketStage::Closure].deactivate();
        self.work_buckets[WorkBucketStage::RefClosure].deactivate();
        self.work_buckets[WorkBucketStage::RefForwarding].deactivate();
        self.work_buckets[WorkBucketStage::Release].deactivate();
        self.work_buckets[WorkBucketStage::Final].deactivate();
    }

    pub fn add_coordinator_work(&self, work: impl CoordinatorWork<C>, worker: &Worker<C>) {
        worker
            .sender
            .send(CoordinatorMessage::Work(box work))
            .unwrap();
    }

    #[inline]
    fn pop_scheduable_work(&self, worker: &Worker<C>) -> Option<(Box<dyn Work<C>>, bool)> {
        if let Some(work) = worker.local_work_bucket.poll() {
            return Some((work, worker.local_work_bucket.is_empty()));
        }
        for work_bucket in self.work_buckets.values() {
            if let Some(work) = work_bucket.poll() {
                return Some((work, work_bucket.is_empty()));
            }
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
            if self.worker_group().all_parked() {
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
        let coordinator_worker = self.coordinator_worker.as_ref().unwrap().read().unwrap();
        coordinator_worker.stat.enable();
    }

    pub fn statistics(&self) -> HashMap<String, String> {
        let mut summary = SchedulerStat::default();
        for worker in &self.worker_group().workers {
            summary.merge(&worker.stat);
        }
        let coordinator_worker = self.coordinator_worker.as_ref().unwrap().read().unwrap();
        summary.merge(&coordinator_worker.stat);
        summary.harness_stat()
    }
}

#[cfg(test)]
mod tests {
    /* An implementation of parallel quicksort */
    use crate::scheduler::*;
    use crate::util::opaque_pointer::*;
    use lazy_static::lazy_static;
    use rand::{thread_rng, Rng};
    use std::sync::Arc;

    /// A work-packet to (quick)sort a slice of array
    struct Sort(&'static mut [usize]);

    impl Work<()> for Sort {
        fn do_work(&mut self, worker: &mut Worker<()>, _context: &'static ()) {
            if self.0.len() <= 1 {
                return; /* Do nothing */
            }
            worker.scheduler().work_buckets[WorkBucketStage::Unconstrained]
                .add(Partition(unsafe { &mut *(self.0 as *mut _) }));
        }
    }

    /// A work-packet to do array partition
    ///
    /// Recursively generates `Sort` work for partitioned sub-arrays.
    struct Partition(&'static mut [usize]);

    impl Work<()> for Partition {
        fn do_work(&mut self, worker: &mut Worker<()>, _context: &'static ()) {
            assert!(self.0.len() >= 2);

            // 1. Partition

            let pivot: usize = self.0[0];
            let le = self
                .0
                .iter()
                .skip(1)
                .filter(|v| **v <= pivot)
                .copied()
                .collect::<Vec<_>>();
            let gt = self
                .0
                .iter()
                .skip(1)
                .filter(|v| **v > pivot)
                .copied()
                .collect::<Vec<_>>();

            let pivot_index = le.len();
            for (i, v) in le.iter().enumerate() {
                self.0[i] = *v;
            }
            self.0[pivot_index] = pivot;
            for (i, v) in gt.iter().enumerate() {
                self.0[pivot_index + i + 1] = *v;
            }

            // 2. Create two `Sort` work packets

            let left: &'static mut [usize] =
                unsafe { &mut *(&mut self.0[..pivot_index] as *mut _) };
            let right: &'static mut [usize] =
                unsafe { &mut *(&mut self.0[pivot_index + 1..] as *mut _) };

            worker.scheduler().work_buckets[WorkBucketStage::Unconstrained].add(Sort(left));
            worker.scheduler().work_buckets[WorkBucketStage::Unconstrained].add(Sort(right));
        }
    }

    lazy_static! {
        static ref SCHEDULER: Arc<Scheduler<()>> = Scheduler::new();
    }

    const NUM_WORKERS: usize = 16;

    fn random_array(size: usize) -> Box<[usize]> {
        let mut rng = thread_rng();
        (0..size).map(|_| rng.gen()).collect()
    }

    #[test]
    fn quicksort() {
        let data: &'static mut [usize] = Box::leak(random_array(1000));

        // println!("Original: {:?}", data);

        SCHEDULER.initialize(NUM_WORKERS, &(), VMThread::UNINITIALIZED);
        SCHEDULER.enable_stat();
        SCHEDULER.work_buckets[WorkBucketStage::Unconstrained]
            .add(Sort(unsafe { &mut *(data as *mut _) }));
        SCHEDULER.wait_for_completion();

        // println!("Sorted: {:?}", data);

        println!("{:?}", SCHEDULER.statistics());

        assert!(data.is_sorted());

        let _data = unsafe { Box::from_raw(data) };
    }
}

pub type MMTkScheduler<VM> = Scheduler<MMTK<VM>>;

impl<VM: VMBinding> MMTkScheduler<VM> {
    pub fn notify_mutators_paused(&self, mmtk: &'static MMTK<VM>) {
        mmtk.plan.base().control_collector_context.clear_request();
        debug_assert!(!self.work_buckets[WorkBucketStage::Prepare].is_activated());
        println!("Activate WorkBucketStage::Prepare");
        self.work_buckets[WorkBucketStage::Prepare].activate();
        let _guard = self.worker_monitor.0.lock().unwrap();
        self.worker_monitor.1.notify_all();
    }
}
