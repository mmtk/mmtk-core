use crate::scheduler::GCWorkScheduler;
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
                gc_scheduled: false,
            }),
            request_flag: AtomicBool::new(false),
            scheduler,
            phantom: PhantomData,
        }
    }

    /// Request a GC.  Called by mutators when polling (during allocation) and when handling user
    /// GC requests (e.g. `System.gc();` in Java);
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

    /// Returns true if GC has been scheduled.
    pub fn is_gc_scheduled(&self) -> bool {
        let guard = self.request_sync.lock().unwrap();
        guard.gc_scheduled
    }

    /// Clear the "GC requested" flag so that mutators can trigger the next GC.
    /// Called by a GC worker when all mutators have come to a stop.
    pub fn clear_request(&self) {
        let _guard = self.request_sync.lock().unwrap();
        self.request_flag.store(false, Ordering::Relaxed);
    }

    /// Called by a GC worker when a GC has finished.
    /// This will check the `request_flag` again and schedule the next GC.
    ///
    /// Note that this may schedule the next GC immediately if
    /// 1.  The plan is concurrent, and a mutator triggered another GC while the current GC was
    ///     still running (between `clear_request` and `on_gc_finished`), or
    /// 2.  After the invocation of `resume_mutators`, a mutator runs so fast that it
    ///     exhausted the heap, or called `handle_user_collection_request`, before this function
    ///     is called.
    pub fn on_gc_finished(&self) {
        let mut guard = self.request_sync.lock().unwrap();
        guard.gc_scheduled = false;

        self.try_schedule_collection(&mut *guard);
    }

    fn try_schedule_collection(&self, sync: &mut RequestSync) {
        // Do not schedule collection if a GC is still in progress.
        // When the GC finishes, a GC worker will call `on_gc_finished` and check `request_flag`
        // again.
        if self.request_flag.load(Ordering::Relaxed) && !sync.gc_scheduled {
            self.scheduler.schedule_collection();

            sync.gc_scheduled = true;

            // Note: We do not clear `request_flag` now.  It will be cleared by `clear_request`
            // after all mutators have stopped.
        }
    }
}
