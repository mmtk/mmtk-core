use super::ParallelCollectorGroup;

use std::cell::UnsafeCell;
use std::sync::{Mutex, Condvar};
use std::sync::atomic::{AtomicBool, Ordering};

use ::vm::Scheduling;
use ::vm::VMScheduling;

use ::plan::{Plan, ParallelCollector};
use ::plan::selected_plan::SelectedPlan;

struct RequestSync {
    thread_id: usize,
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext<'a> {
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,

    pub workers: UnsafeCell<ParallelCollectorGroup<<SelectedPlan<'a> as Plan>::CollectorT>>,
    request_flag: AtomicBool,
}

unsafe impl<'a> Sync for ControllerCollectorContext<'a> {}

impl<'a> ControllerCollectorContext<'a> {
    pub fn new() -> Self {
        ControllerCollectorContext {
            request_sync: Mutex::new(RequestSync {
                thread_id: 0,
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),

            workers: UnsafeCell::new(ParallelCollectorGroup::<<SelectedPlan<'a> as Plan>::CollectorT>::new()),
            request_flag: AtomicBool::new(false),
        }
    }

    pub fn run(&self, thread_id: usize) {
        {
            self.request_sync.lock().unwrap().thread_id = thread_id;
        }

        // Safe provided that we don't hold a &mut to this struct
        // before executing run()
        let workers = unsafe { &*self.workers.get() };

        loop {
            self.wait_for_request();
            VMScheduling::stop_all_mutators(thread_id);
            self.clear_request();
            println!("Doing collection");

            workers.trigger_cycle();

            workers.wait_for_cycle();

            VMScheduling::resume_mutators(thread_id);
            println!("Finished!");
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