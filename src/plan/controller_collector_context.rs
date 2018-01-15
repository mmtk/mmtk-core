use super::ParallelCollectorGroup;

use std::sync::{Mutex, Condvar};
use ::vm::Scheduling;
use ::vm::VMScheduling;

use std::mem::transmute;

use ::plan::plan::Plan;
use ::plan::selected_plan;

struct RequestSync {
    thread_id: usize,
    request_flag: bool,
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext<'a> {
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,

    workers: ParallelCollectorGroup<selected_plan::SelectedCollector<'a>>,
}

impl<'a> ControllerCollectorContext<'a> {
    pub fn new() -> Self {
        ControllerCollectorContext {
            request_sync: Mutex::new(RequestSync {
                thread_id: 0,
                request_flag: false,
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),

            workers: ParallelCollectorGroup::<selected_plan::SelectedCollector>::new(),
        }
    }

    pub fn run(&self, thread_id: usize) {
        {
            self.request_sync.lock().unwrap().thread_id = thread_id;
        }
        loop {
            self.wait_for_request();
            VMScheduling::stop_all_mutators(thread_id);
            self.clear_request();
            println!("Doing collection");

            self.workers.trigger_cycle();

            self.workers.wait_for_cycle();

            VMScheduling::resume_mutators(thread_id);
            println!("Finished!");
        }
    }

    pub fn request(&self) {
        // Required to "punch through" the Mutex. May invoke undefined behaviour. :(
        // NOTE: Strictly speaking we can remove this entire block while maintaining correctness.
        #[allow(mutable_transmutes)]
        unsafe {
            let unsafe_handle = transmute::<&Self, &mut Self>(self).request_sync.get_mut().unwrap();
            if unsafe_handle.request_flag {
                return;
            }
        }

        let mut guard = self.request_sync.lock().unwrap();
        if !guard.request_flag {
            guard.request_flag = true;
            guard.request_count += 1;
            self.request_condvar.notify_all();
        }
    }

    pub fn clear_request(&self) {
        let mut guard = self.request_sync.lock().unwrap();
        guard.request_flag = false;
    }

    fn wait_for_request(&self) {
        let mut guard = self.request_sync.lock().unwrap();
        guard.last_request_count += 1;
        while guard.last_request_count == guard.request_count {
            guard = self.request_condvar.wait(guard).unwrap();
        }
    }
}