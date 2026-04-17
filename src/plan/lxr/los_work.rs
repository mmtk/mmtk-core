use crate::plan::lxr::LazySweepingJobsCounter;
use crate::scheduler::{GCWork, GCWorker};
use crate::vm::VMBinding;
use crate::MMTK;

pub(crate) struct RCSweepMatureAfterSATBLOS {
    _counter: LazySweepingJobsCounter,
}

impl RCSweepMatureAfterSATBLOS {
    pub(crate) fn new(counter: LazySweepingJobsCounter) -> Self {
        Self { _counter: counter }
    }
}

impl<VM: VMBinding> GCWork<VM> for RCSweepMatureAfterSATBLOS {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let los = mmtk.get_plan().common().get_los();
        los.sweep_rc_mature_objects_after_satb(&|o| !(!los.is_marked(o) && los.rc.count(o) != 0));
    }
}
