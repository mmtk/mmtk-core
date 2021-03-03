use crate::util::analysis::RtAnalysis;
use crate::util::statistics::counter::EventCounter;
use crate::vm::VMBinding;
use crate::MMTK;
use std::sync::{Arc, Mutex};

/**
 * Simple analysis routine that counts the number of collections over course of program execution
 */
pub struct GcCounter {
    running: bool,
    counter: Arc<Mutex<EventCounter>>,
}

impl GcCounter {
    pub fn new(running: bool, counter: Arc<Mutex<EventCounter>>) -> Self {
        Self { running, counter }
    }
}

impl<VM: VMBinding> RtAnalysis<VM> for GcCounter {
    fn gc_hook(&mut self, _mmtk: &'static MMTK<VM>) {
        if self.running {
            // The analysis routine simply updates the counter when the allocation hook is called
            self.counter.lock().unwrap().inc();
        }
    }

    fn set_running(&mut self, running: bool) {
        self.running = running;
    }
}
