//! This module holds work packets for `CommonPlan` and `BasePlan`, or other work packets not
//! directly related to scheduling.

use crate::{plan::global::CommonPlan, scheduler::GCWork, vm::VMBinding};

pub(super) struct SetCommonPlanUnlogBits<VM: VMBinding> {
    pub common_plan: &'static CommonPlan<VM>,
}

impl<VM: VMBinding> GCWork<VM> for SetCommonPlanUnlogBits<VM> {
    fn do_work(
        &mut self,
        _worker: &mut crate::scheduler::GCWorker<VM>,
        _mmtk: &'static crate::MMTK<VM>,
    ) {
        self.common_plan.set_side_log_bits();
    }
}

pub(super) struct ClearCommonPlanUnlogBits<VM: VMBinding> {
    pub common_plan: &'static CommonPlan<VM>,
}

impl<VM: VMBinding> GCWork<VM> for ClearCommonPlanUnlogBits<VM> {
    fn do_work(
        &mut self,
        _worker: &mut crate::scheduler::GCWorker<VM>,
        _mmtk: &'static crate::MMTK<VM>,
    ) {
        self.common_plan.clear_side_log_bits();
    }
}
