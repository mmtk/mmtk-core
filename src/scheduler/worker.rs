use super::stat::WorkerLocalStat;
use super::work_bucket::*;
use super::*;
use crate::mmtk::MMTK;
use crate::util::OpaquePointer;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Weak};

/// This struct will be accessed during trace_object(), which is performance critical.
/// However, we do not know its concrete type as the plan and its copy context is dynamically selected.
/// Instead use a void* type to store it, and during trace_object() we cast it to the correct copy context type.
#[derive(Copy, Clone)]
pub struct WorkerLocalPtr {
    data: *mut c_void,
    // Save the type name for debug builds, so we can later do type check
    #[cfg(debug_assertions)]
    ty: &'static str,
}
impl WorkerLocalPtr {
    pub const UNINITIALIZED: Self = WorkerLocalPtr {
        data: std::ptr::null_mut(),
        #[cfg(debug_assertions)]
        ty: "uninitialized",
    };

    pub fn new<W: WorkerLocal>(worker_local: W) -> Self {
        WorkerLocalPtr {
            data: Box::into_raw(Box::new(worker_local)) as *mut c_void,
            #[cfg(debug_assertions)]
            ty: std::any::type_name::<W>(),
        }
    }

    /// # Safety
    /// The user needs to guarantee that the type supplied here is the same type used to create this pointer.
    pub unsafe fn as_type<W: WorkerLocal>(&mut self) -> &mut W {
        #[cfg(debug_assertions)]
        debug_assert_eq!(self.ty, std::any::type_name::<W>());
        &mut *(self.data as *mut W)
    }
}

const LOCALLY_CACHED_WORKS: usize = 1;

pub struct Worker<C: Context> {
    pub tls: OpaquePointer,
    pub ordinal: usize,
    pub parked: AtomicBool,
    scheduler: Arc<Scheduler<C>>,
    local: WorkerLocalPtr,
    pub local_work: WorkBucket<C>,
    pub sender: Sender<CoordinatorMessage<C>>,
    pub stat: WorkerLocalStat,
    context: Option<&'static C>,
    is_coordinator: bool,
    local_work_buffer: Vec<(WorkBucketStage, Box<dyn Work<C>>)>,
}

unsafe impl<C: Context> Sync for Worker<C> {}
unsafe impl<C: Context> Send for Worker<C> {}

pub type GCWorker<VM> = Worker<MMTK<VM>>;

impl<C: Context> Worker<C> {
    pub fn new(ordinal: usize, scheduler: Weak<Scheduler<C>>, is_coordinator: bool) -> Self {
        let scheduler = scheduler.upgrade().unwrap();
        Self {
            tls: OpaquePointer::UNINITIALIZED,
            ordinal,
            parked: AtomicBool::new(true),
            local: WorkerLocalPtr::UNINITIALIZED,
            local_work: WorkBucket::new(true, scheduler.worker_monitor.clone()),
            sender: scheduler.channel.0.clone(),
            scheduler,
            stat: Default::default(),
            context: None,
            is_coordinator,
            local_work_buffer: Vec::with_capacity(LOCALLY_CACHED_WORKS),
        }
    }

    #[inline]
    pub fn add_work(&mut self, bucket: WorkBucketStage, work: impl Work<C>) {
        if !self.scheduler().work_buckets[bucket].is_activated() {
            self.scheduler.work_buckets[bucket].add_with_priority(1000, box work);
            return;
        }
        self.local_work_buffer.push((bucket, box work));
        if self.local_work_buffer.len() > LOCALLY_CACHED_WORKS {
            self.flush();
        }
    }

    #[cold]
    fn flush(&mut self) {
        let mut buffer = Vec::with_capacity(LOCALLY_CACHED_WORKS);
        std::mem::swap(&mut buffer, &mut self.local_work_buffer);
        for (bucket, work) in buffer {
            self.scheduler.work_buckets[bucket].add_with_priority(1000, work);
        }
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::SeqCst)
    }

    pub fn is_coordinator(&self) -> bool {
        self.is_coordinator
    }

    pub fn scheduler(&self) -> &Scheduler<C> {
        &self.scheduler
    }

    /// # Safety
    /// The user needs to guarantee that the type supplied here is the same type used to create this pointer.
    #[inline]
    pub unsafe fn local<W: 'static + WorkerLocal>(&mut self) -> &mut W {
        self.local.as_type::<W>()
    }

    pub fn set_local(&mut self, local: WorkerLocalPtr) {
        self.local = local;
    }

    pub fn init(&mut self, tls: OpaquePointer) {
        self.tls = tls;
    }

    pub fn do_work(&'static mut self, mut work: impl Work<C>) {
        work.do_work(self, self.context.unwrap());
    }

    pub fn run(&mut self, context: &'static C) {
        self.context = Some(context);
        self.parked.store(false, Ordering::SeqCst);
        loop {
            while let Some((bucket, mut work)) = self.local_work_buffer.pop() {
                debug_assert!(self.scheduler.work_buckets[bucket].is_activated());
                work.do_work_with_stat(self, context);
            }
            let mut work = self.scheduler().poll(self);
            debug_assert!(!self.is_parked());
            work.do_work_with_stat(self, context);
        }
    }
}

pub struct WorkerGroup<C: Context> {
    pub workers: Vec<Worker<C>>,
}

impl<C: Context> WorkerGroup<C> {
    pub fn new(workers: usize, scheduler: Weak<Scheduler<C>>) -> Arc<Self> {
        Arc::new(Self {
            workers: (0..workers)
                .map(|i| Worker::new(i, scheduler.clone(), false))
                .collect(),
        })
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn parked_workers(&self) -> usize {
        self.workers.iter().filter(|w| w.is_parked()).count()
    }

    pub fn all_parked(&self) -> bool {
        self.parked_workers() == self.worker_count()
    }

    pub fn spawn_workers(&'static self, tls: OpaquePointer, context: &'static C) {
        for i in 0..self.worker_count() {
            let worker = &self.workers[i];
            C::spawn_worker(worker, tls, context);
        }
    }
}
