use crate::util::analysis::RtAnalysis;
use crate::util::statistics::counter::EventCounter;
use std::sync::{Arc, Mutex};

/**
 * Simple analysis routine that counts the number of objects allocated
 */
pub struct ObjectCounter {
    counter: Arc<Mutex<EventCounter>>,
}

impl ObjectCounter {
    pub fn new(counter: Arc<Mutex<EventCounter>>) -> Self {
        Self { counter }
    }
}

// Since no special arguments are required, the routine uses unit/void for implementing the trait
impl RtAnalysis<(), ()> for ObjectCounter {
    fn alloc_hook(&mut self, _args: ()) {
        self.counter.lock().unwrap().inc();
    }
}
