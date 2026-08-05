//! This module holds work packets for `CommonPlan` and `BasePlan`, or other work packets not
//! directly related to scheduling.

use std::marker::PhantomData;

use crate::{scheduler::GCWork, vm::VMBinding};

#[derive(Default)]
pub(super) struct SetCommonPlanUnlogBits<VM: VMBinding> {
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> GCWork<VM> for SetCommonPlanUnlogBits<VM> {
    fn do_work(
        &mut self,
        _worker: &mut crate::scheduler::GCWorker<VM>,
        mmtk: &'static crate::MMTK<VM>,
    ) {
        mmtk.get_plan().common().set_side_log_bits();
    }
}

#[derive(Default)]
pub(super) struct ClearCommonPlanUnlogBits<VM: VMBinding> {
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding> GCWork<VM> for ClearCommonPlanUnlogBits<VM> {
    fn do_work(
        &mut self,
        _worker: &mut crate::scheduler::GCWorker<VM>,
        mmtk: &'static crate::MMTK<VM>,
    ) {
        mmtk.get_plan().common().clear_side_log_bits();
    }
}
