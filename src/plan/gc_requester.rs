use crate::scheduler::GCWorkScheduler;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// This data structure lets mutators trigger GC.
pub struct GCRequester<VM: VMBinding> {
    /// Set by mutators to trigger GC.  It is atomic so that mutators can check if GC has already
    /// been requested efficiently in `poll` without acquiring any mutex.
    request_flag: AtomicBool,
    scheduler: Arc<GCWorkScheduler<VM>>,
}

impl<VM: VMBinding> GCRequester<VM> {
    pub fn new(scheduler: Arc<GCWorkScheduler<VM>>) -> Self {
        GCRequester {
            request_flag: AtomicBool::new(false),
            scheduler,
        }
    }

    /// Request a GC.  Called by mutators when polling (during allocation) and when handling user
    /// GC requests (e.g. `System.gc();` in Java).
    pub fn request(&self) {
        if self.request_flag.load(Ordering::Relaxed) {
            return;
        }

        if !self.request_flag.swap(true, Ordering::Relaxed) {
            // `GCWorkScheduler::request_schedule_collection` needs to hold a mutex to communicate
            // with GC workers, which is expensive for functions like `poll`.  We use the atomic
            // flag `request_flag` to elide the need to acquire the mutex in subsequent calls.
            self.scheduler.request_schedule_collection();
        }
    }

    /// Clear the "GC requested" flag so that mutators can trigger the next GC.
    /// Called by a GC worker when all mutators have come to a stop.
    pub fn clear_request(&self) {
        self.request_flag.store(false, Ordering::Relaxed);
    }
}
