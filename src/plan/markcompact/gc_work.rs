use super::global::MarkCompact;
use crate::policy::markcompactspace::MarkCompactSpace;
use crate::policy::markcompactspace::{TRACE_KIND_FORWARD, TRACE_KIND_MARK};
use crate::scheduler::gc_work::PlanProcessEdges;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::scheduler::WorkBucketStage;
use crate::vm::ActivePlan;
use crate::vm::Scanning;
use crate::vm::VMBinding;
use crate::MMTK;
use std::marker::PhantomData;

/// iterate through the heap and calculate the new location of live objects
pub struct CalculateForwardingAddress<VM: VMBinding> {
    mc_space: &'static MarkCompactSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for CalculateForwardingAddress<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.mc_space.calculate_forwarding_pointer();
    }
}

impl<VM: VMBinding> CalculateForwardingAddress<VM> {
    pub fn new(mc_space: &'static MarkCompactSpace<VM>) -> Self {
        Self { mc_space }
    }
}

/// create another round of root scanning work packets
/// to update object references
pub struct UpdateReferences<VM: VMBinding> {
    plan: *const MarkCompact<VM>,
    p: PhantomData<VM>,
}

unsafe impl<VM: VMBinding> Send for UpdateReferences<VM> {}

impl<VM: VMBinding> GCWork<VM> for UpdateReferences<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        // The following needs to be done right before the second round of root scanning
        VM::VMScanning::prepare_for_roots_re_scanning();
        mmtk.state.prepare_for_stack_scanning();
        // Prepare common and base spaces for the 2nd round of transitive closure
        let plan_mut = unsafe { &mut *(self.plan as *mut MarkCompact<VM>) };
        plan_mut.common.release(worker.tls, true);
        plan_mut.common.prepare(worker.tls, true);
        #[cfg(feature = "extreme_assertions")]
        mmtk.edge_logger.reset();

        // We do two passes of transitive closures. We clear the live bytes from the first pass.
        #[cfg(feature = "count_live_bytes_in_gc")]
        mmtk.scheduler
            .worker_group
            .get_and_clear_worker_live_bytes();

        for mutator in VM::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::SecondRoots].add(ScanMutatorRoots::<
                MarkCompactForwardingGCWorkContext<VM>,
            >(mutator));
        }

        mmtk.scheduler.work_buckets[WorkBucketStage::SecondRoots]
            .add(ScanVMSpecificRoots::<MarkCompactForwardingGCWorkContext<VM>>::new());
    }
}

impl<VM: VMBinding> UpdateReferences<VM> {
    pub fn new(plan: &MarkCompact<VM>) -> Self {
        Self {
            plan,
            p: PhantomData,
        }
    }
}

/// compact live objects based on forwarding pointers calculated before
pub struct Compact<VM: VMBinding> {
    mc_space: &'static MarkCompactSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for Compact<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.mc_space.compact();
    }
}

impl<VM: VMBinding> Compact<VM> {
    pub fn new(mc_space: &'static MarkCompactSpace<VM>) -> Self {
        Self { mc_space }
    }
}

/// Marking trace
pub type MarkingProcessEdges<VM> = PlanProcessEdges<VM, MarkCompact<VM>, TRACE_KIND_MARK>;
/// Forwarding trace
pub type ForwardingProcessEdges<VM> = PlanProcessEdges<VM, MarkCompact<VM>, TRACE_KIND_FORWARD>;

pub struct MarkCompactGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MarkCompactGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MarkCompact<VM>;
    type DefaultProcessEdges = MarkingProcessEdges<VM>;
    type PinningProcessEdges = UnsupportedProcessEdges<VM>;
}

pub struct MarkCompactForwardingGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MarkCompactForwardingGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MarkCompact<VM>;
    type DefaultProcessEdges = ForwardingProcessEdges<VM>;
    type PinningProcessEdges = UnsupportedProcessEdges<VM>;
}
