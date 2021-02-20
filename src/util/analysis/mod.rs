use crate::scheduler::*;
use crate::vm::VMBinding;
use crate::MMTK;
use std::sync::{Arc, Mutex};

pub mod gc_count;
pub mod obj_num;
pub mod obj_size;

/**
 * This trait exposes hooks for developers to implement their own analysis routines.
 *
 * Most traits would want to hook into the `Stats` and counters provided by the MMTk
 * framework that are exposed to the Harness.
 *
 * The arguments for the hooks should be sufficient, however, if one wishes to add
 * other arguments, then they can create an analysis routine specific function and
 * invoke it in its respective place.
 */
pub trait RtAnalysis<VM: VMBinding> {
    fn alloc_hook(&mut self, _size: usize, _align: usize, _offset: isize) {}
    fn gc_hook(&mut self, _mmtk: &'static MMTK<VM>) {}
    fn set_running(&mut self, running: bool);
}

#[derive(Default)]
pub struct GcHookWork;

impl<VM: VMBinding> GCWork<VM> for GcHookWork {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let base = &mmtk.plan.base();
        base.analysis_manager.lock().unwrap().gc_hook(mmtk);
    }
}

// The AnalysisManager essentially acts as a proxy for all analysis routines made.
// The framwework uses the AnalysisManager to call hooks for analysis routines.
#[derive(Default)]
pub struct AnalysisManager<VM: VMBinding> {
    routines: Mutex<Vec<Arc<Mutex<dyn RtAnalysis<VM> + Send>>>>,
}

impl<VM: VMBinding> AnalysisManager<VM> {
    pub fn new() -> Self {
        AnalysisManager {
            routines: Mutex::new(vec![]),
        }
    }

    pub fn add_analysis_routine(&mut self, routine: Arc<Mutex<dyn RtAnalysis<VM> + Send>>) {
        let mut routines = self.routines.lock().unwrap();
        routines.push(routine.clone());
    }

    pub fn alloc_hook(&self, size: usize, align: usize, offset: isize) {
        let routines = self.routines.lock().unwrap();
        for r in &*routines {
            r.lock().unwrap().alloc_hook(size, align, offset);
        }
    }

    pub fn gc_hook(&self, mmtk: &'static MMTK<VM>) {
        let routines = self.routines.lock().unwrap();
        for r in &*routines {
            r.lock().unwrap().gc_hook(mmtk);
        }
    }
}
