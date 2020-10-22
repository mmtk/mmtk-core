use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, Arc};
use std::marker::PhantomData;

use crate::mmtk::MMTK;

use crate::plan::Plan;

use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::scheduler::*;

struct RequestSync {
    tls: OpaquePointer,
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext<VM: VMBinding> {
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,

    pub mmtk: &'static MMTK<VM>,
    pub scheduler: Arc<MMTkScheduler<VM>>,
    request_flag: AtomicBool,
    phantom: PhantomData<VM>,
}

unsafe impl <VM: VMBinding> Sync for ControllerCollectorContext<VM> {}

impl <VM: VMBinding> ControllerCollectorContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        ControllerCollectorContext {
            request_sync: Mutex::new(RequestSync {
                tls: OpaquePointer::UNINITIALIZED,
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),

            scheduler: mmtk.scheduler.clone(),
            mmtk,
            request_flag: AtomicBool::new(false),
            phantom: PhantomData,
        }
    }

    pub fn run(&self, tls: OpaquePointer) {
        {
            self.request_sync.lock().unwrap().tls = tls;
        }

        // Safe provided that we don't hold a &mut to this struct
        // before executing run()
        // let workers = unsafe { &*self.workers.get() };

        loop {
            debug!("[STWController: Waiting for request...]");
            self.wait_for_request();
            debug!("[STWController: Request recieved.]");

            // debug!("[STWController: Stopping the world...]");
            // VM::VMCollection::stop_all_mutators(tls);

            // For heap growth logic
            // FIXME: This is not used. However, we probably want to set a 'user_triggered' flag
            // when GC is requested.
            // let user_triggered_collection: bool = SelectedPlan::is_user_triggered_collection();

            // self.clear_request();

            // debug!("[STWController: Triggering worker threads...]");
            // self.scheduler.mutators_stopped();
            self.mmtk.plan.schedule_collection(&self.scheduler);

            self.scheduler.wait_for_completion();
            debug!("[STWController: Worker threads complete!]");
            // debug!("[STWController: Resuming mutators...]");
            // VM::VMCollection::resume_mutators(tls);
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
