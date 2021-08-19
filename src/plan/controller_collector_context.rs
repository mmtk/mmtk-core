//! The GC controller thread.

use crate::scheduler::gc_work::{ConcurrentWorkEnd, ScheduleCollection};
use crate::scheduler::*;
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::sync::{Arc, Condvar, Mutex};

use super::immix::gc_work::{ImmixProcessEdges, TraceKind};

struct RequestSync {
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext<VM: VMBinding> {
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,
    scheduler: RwLock<Option<Arc<GCWorkScheduler<VM>>>>,
    request_flag: AtomicBool,
    pub concurrent: AtomicBool,
    phantom: PhantomData<VM>,
}

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
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),
            scheduler: RwLock::new(None),
            request_flag: AtomicBool::new(false),
            concurrent: AtomicBool::new(false),
            phantom: PhantomData,
        }
    }

    pub fn init(&self, scheduler: &Arc<GCWorkScheduler<VM>>) {
        let mut scheduler_guard = self.scheduler.write().unwrap();
        debug_assert!(scheduler_guard.is_none());
        *scheduler_guard = Some(scheduler.clone());
    }

    pub fn run(&self, tls: VMWorkerThread) {
        loop {
            debug!("[STWController: Waiting for request...]");
            self.wait_for_request();
            println!(
                "[STWController: Request recieved.] {}",
                self.concurrent.load(Ordering::SeqCst)
            );

            // For heap growth logic
            // FIXME: This is not used. However, we probably want to set a 'user_triggered' flag
            // when GC is requested.
            // let user_triggered_collection: bool = SelectedPlan::is_user_triggered_collection();

            let scheduler = self.scheduler.read().unwrap();
            let scheduler = scheduler.as_ref().unwrap();
            scheduler.initialize_worker(tls);
            scheduler.set_initializer(Some(ScheduleCollection(
                self.concurrent.load(Ordering::SeqCst),
            )));
            scheduler.wait_for_completion();
            debug!("[STWController: Worker threads complete!]");
        }
    }

    pub fn terminate_concurrent_gc(&self) {
        println!("terminate_concurrent_gc");
        let scheduler = self.scheduler.read().unwrap();
        let scheduler = scheduler.as_ref().unwrap();
        scheduler.work_buckets[WorkBucketStage::PreClosure].add(ConcurrentWorkEnd::<
            ImmixProcessEdges<VM, { TraceKind::Fast }>,
        >::new());
    }

    pub fn request(&self, concurrent: bool) {
        if self.request_flag.load(Ordering::Relaxed) {
            return;
        }

        let mut guard = self.request_sync.lock().unwrap();
        if !self.request_flag.load(Ordering::Relaxed) {
            self.request_flag.store(true, Ordering::Relaxed);
            self.concurrent.store(concurrent, Ordering::SeqCst);
            println!("concurrent = {}", self.concurrent.load(Ordering::SeqCst));
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
