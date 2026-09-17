use super::global::OVC;
use crate::plan::tracing::{PlanTrace, UnsupportedTrace};
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::ovc::{OVCSpace, TRACE_KIND_FORWARD_ROOT, TRACE_KIND_MARK};
use crate::scheduler::gc_work::*;
use crate::scheduler::{GCWork, GCWorker, WorkBucketStage};
use crate::vm::{ActivePlan, Scanning, VMBinding};
use crate::MMTK;
use std::marker::{PhantomData, Send};

/// Generate more packets by calling a method on [`OVCSpace`].
pub struct GenerateWork<VM: VMBinding, F: Fn(&'static OVCSpace<VM>) + Send + 'static> {
    ovc_space: &'static OVCSpace<VM>,
    f: F,
}

impl<VM: VMBinding, F: Fn(&'static OVCSpace<VM>) + Send + 'static> GCWork<VM>
    for GenerateWork<VM, F>
{
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        (self.f)(self.ovc_space);
    }
}

impl<VM: VMBinding, F: Fn(&'static OVCSpace<VM>) + Send + 'static> GenerateWork<VM, F> {
    pub fn new(ovc_space: &'static OVCSpace<VM>, f: F) -> Self {
        Self { ovc_space, f }
    }
}

/// Create another round of root scanning work packets
/// to update object references.
pub struct UpdateReferences<VM: VMBinding> {
    p: PhantomData<VM>,
}

unsafe impl<VM: VMBinding> Send for UpdateReferences<VM> {}

impl<VM: VMBinding> GCWork<VM> for UpdateReferences<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        // The following needs to be done right before the second round of root scanning
        VM::VMScanning::prepare_for_roots_re_scanning();
        mmtk.state.prepare_for_stack_scanning();
        #[cfg(feature = "extreme_assertions")]
        mmtk.slot_logger.reset();

        for mutator in VM::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::SecondRoots]
                .add(ScanMutatorRoots::<OVCForwardingWorkContext<VM>>(mutator));
        }

        mmtk.scheduler.work_buckets[WorkBucketStage::SecondRoots]
            .add(ScanVMSpecificRoots::<OVCForwardingWorkContext<VM>>::new());
    }
}

impl<VM: VMBinding> UpdateReferences<VM> {
    pub fn new() -> Self {
        Self { p: PhantomData }
    }
}

/// Reset the allocator and update references in large object space.
pub struct AfterCompact<VM: VMBinding> {
    ovc_space: &'static OVCSpace<VM>,
    los: &'static LargeObjectSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for AfterCompact<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.ovc_space.after_compact(worker, self.los);
    }
}

impl<VM: VMBinding> AfterCompact<VM> {
    pub fn new(ovc_space: &'static OVCSpace<VM>, los: &'static LargeObjectSpace<VM>) -> Self {
        Self { ovc_space, los }
    }
}

/// Marking trace
pub type MarkingTrace<VM> = PlanTrace<OVC<VM>, TRACE_KIND_MARK>;
/// Forwarding trace
pub type ForwardingTrace<VM> = PlanTrace<OVC<VM>, TRACE_KIND_FORWARD_ROOT>;

pub struct OVCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for OVCWorkContext<VM> {
    type VM = VM;
    type PlanType = OVC<VM>;
    type DefaultTrace = MarkingTrace<VM>;
    type PinningTrace = UnsupportedTrace<VM>;
}

pub struct OVCForwardingWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for OVCForwardingWorkContext<VM> {
    type VM = VM;
    type PlanType = OVC<VM>;
    type DefaultTrace = ForwardingTrace<VM>;
    type PinningTrace = UnsupportedTrace<VM>;
}
