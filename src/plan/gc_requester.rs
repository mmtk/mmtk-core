use crate::scheduler::GCWorkScheduler;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// This data structure lets mutators trigger GC.
pub struct GCRequester<VM: VMBinding> {
    /// An atomic flag outside `RequestSync` so that mutators can check if GC has already been
    /// requested in `poll` without acquiring the mutex.
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
            self.scheduler.request_schedule_collection();
        }
    }

    /// Clear the "GC requested" flag so that mutators can trigger the next GC.
    /// Called by a GC worker when all mutators have come to a stop.
    pub fn clear_request(&self) {
        self.request_flag.store(false, Ordering::Relaxed);
    }
}
