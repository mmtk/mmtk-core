use crate::scheduler::*;
use crate::util::analysis::RtAnalysis;
use crate::util::statistics::counter::EventCounter;
use crate::vm::VMBinding;
use crate::MMTK;
use std::sync::{Arc, Mutex};

/**
 * Simple analysis routine that counts the number of collections over course of program execution
 */
pub struct GcCounter {
    counter: Arc<Mutex<EventCounter>>,
}

impl GcCounter {
    pub fn new(counter: Arc<Mutex<EventCounter>>) -> Self {
        Self { counter }
    }
}

// Since no special arguments are required, the routine uses unit/void for implementing the trait
impl RtAnalysis<(), ()> for GcCounter {
    fn gc_hook(&mut self, _args: ()) {
        self.counter.lock().unwrap().inc();
    }
}

// We could have simply called gc_hook() in schedule_collection(), however it is not advised as
// creating a work packet will be more performant in general as this allows the work to be
// completed asynchronously whenever a worker thread is free.
#[derive(Default)]
pub struct GcCounterWork;

impl GcCounterWork {
    pub fn new() -> Self {
        GcCounterWork
    }
}

impl<VM: VMBinding> GCWork<VM> for GcCounterWork {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let plan = &mmtk.plan;
        plan.base().gc_count.lock().unwrap().gc_hook(());
    }
}
