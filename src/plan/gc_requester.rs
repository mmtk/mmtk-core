use crate::scheduler::gc_work::ScheduleCollection;
use crate::scheduler::{GCWorkScheduler, WorkBucketStage};
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct RequestSync {
    /// Is GC scheduled (but not finished)?
    gc_scheduled: bool,
}

/// This data structure lets mutators trigger GC, and may schedule collection when appropriate.
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
                gc_scheduled: true,
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
        // Note: This is the double-checked locking algorithm.
        // The load has the `Relaxed` order instead of `Acquire` because we only use the flag to
        // remove successive requests, but we don't use it to synchronize other data fields.
        if !self.request_flag.load(Ordering::Relaxed) {
            self.request_flag.store(true, Ordering::Relaxed);

            self.try_schedule_collection(&mut *guard);
        }
    }

    pub fn clear_request(&self) {
        let _guard = self.request_sync.lock().unwrap();
        self.request_flag.store(false, Ordering::Relaxed);
    }

    pub fn on_gc_finished(&self) {
        let mut guard = self.request_sync.lock().unwrap();
        guard.gc_scheduled = false;

        self.try_schedule_collection(&mut *guard);
    }

    fn try_schedule_collection(&self, sync: &mut RequestSync) {
        if self.request_flag.load(Ordering::Relaxed) && !sync.gc_scheduled {
            // Add a ScheduleCollection work packet.  It is the seed of other work packets.
            self.scheduler.work_buckets[WorkBucketStage::Unconstrained].add(ScheduleCollection);

            sync.gc_scheduled = true;

            // Note: We do not clear `request_flag` now.  It will be cleared by `clear_request`
            // after all mutators have stopped.
        }
    }
}
