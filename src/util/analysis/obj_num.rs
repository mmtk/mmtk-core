use crate::util::analysis::RtAnalysis;
use crate::util::statistics::counter::EventCounter;
use crate::vm::VMBinding;
use std::sync::{Arc, Mutex};

/**
 * Simple analysis routine that counts the number of objects allocated
 */
pub struct ObjectCounter {
    running: bool,
    counter: Arc<Mutex<EventCounter>>,
}

impl ObjectCounter {
    pub fn new(running: bool, counter: Arc<Mutex<EventCounter>>) -> Self {
        Self { running, counter }
    }
}

impl<VM: VMBinding> RtAnalysis<VM> for ObjectCounter {
    fn alloc_hook(&mut self, _size: usize, _align: usize, _offset: usize) {
        if self.running {
            // The analysis routine simply updates the counter when the allocation hook is called
            self.counter.lock().unwrap().inc();
        }
    }

    fn set_running(&mut self, running: bool) {
        self.running = running;
    }
}
