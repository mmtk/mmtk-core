use crate::scheduler::gc_work::ScheduleCollection;
use crate::scheduler::{GCWorkScheduler, WorkBucketStage};
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct RequestSync {
    request_count: isize,
    last_request_count: isize,
}

/// GC requester.  This object allows other threads to request (trigger) GC,
/// and the GC coordinator thread waits for GC requests using this object.
pub struct GCRequester<VM: VMBinding> {
    request_sync: Mutex<RequestSync>,
    request_flag: AtomicBool,
    scheduler: Arc<GCWorkScheduler<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> GCRequester<VM> {
    pub fn new(scheduler: Arc<GCWorkScheduler<VM>>) -> Self {
        GCRequester {
            request_sync: Mutex::new(RequestSync {
                request_count: 0,
                last_request_count: -1,
            }),
            request_flag: AtomicBool::new(false),
            scheduler,
            phantom: PhantomData,
        }
    }

    pub fn request(&self) {
        if self.request_flag.load(Ordering::Relaxed) {
            return;
        }

        let mut guard = self.request_sync.lock().unwrap();
        if !self.request_flag.load(Ordering::Relaxed) {
            self.request_flag.store(true, Ordering::Relaxed);
            guard.request_count += 1;

            self.schedule_collection();
        }
    }

    pub fn clear_request(&self) {
        let guard = self.request_sync.lock().unwrap();
        self.request_flag.store(false, Ordering::Relaxed);
        drop(guard);
    }

    fn schedule_collection(&self) {
        // Add a ScheduleCollection work packet.  It is the seed of other work packets.
        self.scheduler.work_buckets[WorkBucketStage::Unconstrained].add(ScheduleCollection);
    }
}
