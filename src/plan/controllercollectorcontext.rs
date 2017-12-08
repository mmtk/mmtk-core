use std::sync::{Mutex, Condvar};

struct RequestSync {
    request_flag: bool,
    request_count: isize,
    last_request_count: isize,
}

pub struct ControllerCollectorContext {
    thread_id: usize,
    request_sync: Mutex<RequestSync>,
    request_condvar: Condvar,
}

impl ControllerCollectorContext {
    pub fn new(thread_id: usize) -> Self {
        ControllerCollectorContext {
            thread_id,
            request_sync: Mutex::new(RequestSync {
                request_flag: false,
                request_count: 0,
                last_request_count: -1,
            }),
            request_condvar: Condvar::new(),
        }
    }

    pub fn run(&mut self) {
        loop {
            self.wait_for_request();
            self.clear_request();
        }
    }

    fn request(&mut self) {
        /*if self.request_flag {
            return;
        }*/

        let mut guard = self.request_sync.lock().unwrap();
        if !guard.request_flag {
            guard.request_flag = true;
            guard.request_count += 1;
            self.request_condvar.notify_all();
        }
    }

    fn clear_request(&mut self) {
        let mut guard = self.request_sync.lock().unwrap();
        guard.request_flag = false;
    }

    fn wait_for_request(&mut self) {
        let mut guard = self.request_sync.lock().unwrap();
        guard.last_request_count += 1;
        while guard.last_request_count == guard.request_count {
            guard = self.request_condvar.wait(guard).unwrap();
        }
    }
}