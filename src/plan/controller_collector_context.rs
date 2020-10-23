use crate::scheduler::gc_works::ScheduleCollection;
use crate::scheduler::*;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::sync::{Arc, Condvar, Mutex};

struct RequestSync {
    tls: OpaquePointer,
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext<VM: VMBinding> {
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,
    scheduler: RwLock<Option<Arc<MMTkScheduler<VM>>>>,
    request_flag: AtomicBool,
    phantom: PhantomData<VM>,
}

unsafe impl<VM: VMBinding> Sync for ControllerCollectorContext<VM> {}

// Clippy says we need this...
impl<VM: VMBinding> Default for ControllerCollectorContext<VM> {
    fn default() -> Self {
        Self::new()
    }
}

impl<VM: VMBinding> ControllerCollectorContext<VM> {
    pub fn new() -> Self {
        ControllerCollectorContext {
            request_sync: Mutex::new(RequestSync {
                tls: OpaquePointer::UNINITIALIZED,
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),
            scheduler: RwLock::new(None),
            request_flag: AtomicBool::new(false),
            phantom: PhantomData,
        }
    }

    pub fn init(&self, scheduler: &Arc<MMTkScheduler<VM>>) {
        let mut scheduler_guard = self.scheduler.write().unwrap();
        debug_assert!(scheduler_guard.is_none());
        *scheduler_guard = Some(scheduler.clone());
    }

    pub fn run(&self, tls: OpaquePointer) {
        {
            self.request_sync.lock().unwrap().tls = tls;
        }

        loop {
            debug!("[STWController: Waiting for request...]");
            self.wait_for_request();
            debug!("[STWController: Request recieved.]");

            // For heap growth logic
            // FIXME: This is not used. However, we probably want to set a 'user_triggered' flag
            // when GC is requested.
            // let user_triggered_collection: bool = SelectedPlan::is_user_triggered_collection();

            let scheduler = self.scheduler.read().unwrap();
            let scheduler = scheduler.as_ref().unwrap();
            scheduler.set_initializer(Some(ScheduleCollection));
            scheduler.wait_for_completion();
            debug!("[STWController: Worker threads complete!]");
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
            self.request_condvar.notify_all();
        }
    }

    pub fn clear_request(&self) {
        let guard = self.request_sync.lock().unwrap();
        self.request_flag.store(false, Ordering::Relaxed);
        drop(guard);
    }

    fn wait_for_request(&self) {
        let mut guard = self.request_sync.lock().unwrap();
        guard.last_request_count += 1;
        while guard.last_request_count == guard.request_count {
            guard = self.request_condvar.wait(guard).unwrap();
        }
    }
}
