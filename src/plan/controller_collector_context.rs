use super::ParallelCollectorGroup;

use std::cell::UnsafeCell;
use std::sync::{Mutex, Condvar};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::vm::Collection;

use crate::plan::Plan;
use crate::plan::selected_plan::SelectedPlan;

use crate::util::OpaquePointer;
use crate::vm::VMBinding;

struct RequestSync {
    tls: OpaquePointer,
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext<VM: VMBinding> {
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,

    pub workers: UnsafeCell<ParallelCollectorGroup<VM, <SelectedPlan<VM> as Plan<VM>>::CollectorT>>,
    request_flag: AtomicBool,
}

unsafe impl<VM: VMBinding> Sync for ControllerCollectorContext<VM> {}

impl<VM: VMBinding> ControllerCollectorContext<VM> {
    pub fn new() -> Self {
        ControllerCollectorContext {
            request_sync: Mutex::new(RequestSync {
                tls: OpaquePointer::UNINITIALIZED,
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),

            workers: UnsafeCell::new(ParallelCollectorGroup::<VM, <SelectedPlan<VM> as Plan<VM>>::CollectorT>::new()),
            request_flag: AtomicBool::new(false),
        }
    }

    pub fn run(&self, tls: OpaquePointer) {
        {
            self.request_sync.lock().unwrap().tls = tls;
        }

        // Safe provided that we don't hold a &mut to this struct
        // before executing run()
        let workers = unsafe { &*self.workers.get() };

        loop {
            debug!("[STWController: Waiting for request...]");
            self.wait_for_request();
            debug!("[STWController: Request recieved.]");
            debug!("[STWController: Stopping the world...]");
            VM::VMCollection::stop_all_mutators(tls);

            // For heap growth logic
            // FIXME: This is not used. However, we probably want to set a 'user_triggered' flag
            // when GC is requested.
            // let user_triggered_collection: bool = SelectedPlan::is_user_triggered_collection();

            self.clear_request();

            debug!("[STWController: Triggering worker threads...]");
            workers.trigger_cycle();

            workers.wait_for_cycle();
            debug!("[STWController: Worker threads complete!]");
            debug!("[STWController: Resuming mutators...]");
            VM::VMCollection::resume_mutators(tls);
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

impl<VM: VMBinding> Default for ControllerCollectorContext<VM> {
    fn default() -> Self {
        Self::new()
    }
}
