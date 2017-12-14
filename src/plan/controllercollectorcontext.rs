use std::sync::{Mutex, Condvar};
use ::vm::scheduler::{stop_all_mutators, resume_mutators};

use std::mem::transmute;

struct RequestSync {
    request_flag: bool,
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext {
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,
}

impl ControllerCollectorContext {
    pub fn new() -> Self {
        ControllerCollectorContext {
            request_sync: Mutex::new(RequestSync {
                request_flag: false,
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),
        }
    }

    pub fn run(&self) {
        loop {
            self.wait_for_request();
            stop_all_mutators();
            self.clear_request();
            // Do collection
            resume_mutators();
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