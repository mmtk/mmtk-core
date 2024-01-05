use crate::scheduler::GCWorkScheduler;
use crate::vm::VMBinding;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct RequestSync {
    /// Are the GC workers already aware that we requested a GC?
    ///
    /// Mutators call `GCRequester::request()` to trigger GC.  It communicates with workers by
    /// calling `GCWorkScheduler::request_schedule_collection`.  Under the hood, it sets a
    /// synchronized variable and notifies one worker.  Conceptually, it is sending a message to GC
    /// workers.
    ///
    /// The purpose of this variable is preventing the message above from being sent while GC
    /// workers are still doing GC.  This is mainly significant for concurrent GC.  In the current
    /// design (inherited from JikesRVM), `request_flag` is cleared when all mutators are
    /// suspended, at which time GC is still in progress.  If `GCRequester::request()` is called
    /// after `request_flag` is cleared but before `on_gc_finished` is called, the mutator will not
    /// send the message.  But GC workers will check `request_flag` when GC finishes, and schedule
    /// the next GC.
    gc_scheduled: bool,
}

/// This data structure lets mutators trigger GC.
pub struct GCRequester<VM: VMBinding> {
    request_sync: Mutex<RequestSync>,
    /// An atomic flag outside `RequestSync` so that mutators can check if GC has already been
    /// requested in `poll` without acquiring the mutex.
    request_flag: AtomicBool,
    scheduler: Arc<GCWorkScheduler<VM>>,
}

impl<VM: VMBinding> GCRequester<VM> {
    pub fn new(scheduler: Arc<GCWorkScheduler<VM>>) -> Self {
        GCRequester {
            request_sync: Mutex::new(RequestSync {
                gc_scheduled: false,
            }),
            request_flag: AtomicBool::new(false),
            scheduler,
        }
    }

    /// Request a GC.  Called by mutators when polling (during allocation) and when handling user
    /// GC requests (e.g. `System.gc();` in Java).
    pub fn request(&self) {
        // Note: This is the double-checked locking algorithm.
        // The load has the `Relaxed` order instead of `Acquire` because we are not doing lazy
        // initialization here.  We are only using this flag to remove successive requests.
        if self.request_flag.load(Ordering::Relaxed) {
            return;
        }

        let mut guard = self.request_sync.lock().unwrap();
        if !self.request_flag.load(Ordering::Relaxed) {
            self.request_flag.store(true, Ordering::Relaxed);

            let should_schedule_gc = self.try_schedule_collection(&mut guard);
            if should_schedule_gc {
                self.scheduler.request_schedule_collection();
                // Note: We do not clear `request_flag` now.  It will be cleared by `clear_request`
                // after all mutators have stopped.
            }
        }
    }

    /// Clear the "GC requested" flag so that mutators can trigger the next GC.
    /// Called by a GC worker when all mutators have come to a stop.
    pub fn clear_request(&self) {
        let _guard = self.request_sync.lock().unwrap();
        self.request_flag.store(false, Ordering::Relaxed);
    }

    /// Called by a GC worker when a GC has finished.
    /// This will check the `request_flag` again and check if we should immediately schedule the
    /// next GC.  If we should, `gc_scheduled` will be set back to `true` and this function will
    /// return `true`.
    pub fn on_gc_finished(&self) -> bool {
        let mut guard = self.request_sync.lock().unwrap();
        guard.gc_scheduled = false;

        self.try_schedule_collection(&mut guard)
    }

    /// Decide whether we should schedule a new collection.  Will transition the state of
    /// `gc_scheduled` from `false` to `true` if we should schedule a new collection.
    ///
    /// Return `true` if the state transition happens.
    fn try_schedule_collection(&self, sync: &mut RequestSync) -> bool {
        // The time to schedule a collection is when `request_flag` is `true` but `gc_scheduled` is
        // `false`.  If `gc_scheduled` is `true` when GC is requested, we do nothing now.  But when
        // the currrent GC finishes, a GC worker will call `on_gc_finished` which clears the
        // `gc_scheduled` flag, and checks the `request_flag` again to trigger the next GC.
        if self.request_flag.load(Ordering::Relaxed) && !sync.gc_scheduled {
            sync.gc_scheduled = true;
            true
        } else {
            false
        }
    }
}
