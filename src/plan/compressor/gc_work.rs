use super::global::Compressor;
use crate::policy::compressor::CompressorSpace;
use crate::policy::compressor::{TRACE_KIND_FORWARD, TRACE_KIND_MARK};
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

/// Iterate through the heap and calculate the new location of live objects.
pub struct CalculateForwardingAddress<VM: VMBinding> {
    compressor_space: &'static CompressorSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for CalculateForwardingAddress<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.compressor_space.calculate_offset_vector();
    }
}

impl<VM: VMBinding> CalculateForwardingAddress<VM> {
    pub fn new(compressor_space: &'static CompressorSpace<VM>) -> Self {
        Self { compressor_space }
    }
}

/// Create another round of root scanning work packets
/// to update object references.
pub struct UpdateReferences<VM: VMBinding> {
    plan: *const Compressor<VM>,
    p: PhantomData<VM>,
}

unsafe impl<VM: VMBinding> Send for UpdateReferences<VM> {}

impl<VM: VMBinding> GCWork<VM> for UpdateReferences<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        // The following needs to be done right before the second round of root scanning
        VM::VMScanning::prepare_for_roots_re_scanning();
        mmtk.state.prepare_for_stack_scanning();
        // Prepare common and base spaces for the 2nd round of transitive closure
        let plan_mut = unsafe { &mut *(self.plan as *mut Compressor<VM>) };
        plan_mut.common.release(worker.tls, true);
        plan_mut.common.prepare(worker.tls, true);
        #[cfg(feature = "extreme_assertions")]
        mmtk.slot_logger.reset();

        // We do two passes of transitive closures. We clear the live bytes from the first pass.
        mmtk.scheduler
            .worker_group
            .get_and_clear_worker_live_bytes();

        for mutator in VM::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::SecondRoots].add(ScanMutatorRoots::<
                CompressorForwardingWorkContext<VM>,
            >(mutator));
        }

        mmtk.scheduler.work_buckets[WorkBucketStage::SecondRoots]
            .add(ScanVMSpecificRoots::<CompressorForwardingWorkContext<VM>>::new());
    }
}

impl<VM: VMBinding> UpdateReferences<VM> {
    pub fn new(plan: &Compressor<VM>) -> Self {
        Self {
            plan,
            p: PhantomData,
        }
    }
}

/// Compact live objects based on the previously-calculated forwarding pointers.
pub struct Compact<VM: VMBinding> {
    compressor_space: &'static CompressorSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for Compact<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.compressor_space.compact(worker);
    }
}

impl<VM: VMBinding> Compact<VM> {
    pub fn new(compressor_space: &'static CompressorSpace<VM>) -> Self {
        Self { compressor_space }
    }
}

/// Marking trace
pub type MarkingProcessEdges<VM> = PlanProcessEdges<VM, Compressor<VM>, TRACE_KIND_MARK>;
/// Forwarding trace
pub type ForwardingProcessEdges<VM> = PlanProcessEdges<VM, Compressor<VM>, TRACE_KIND_FORWARD>;

pub struct CompressorWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for CompressorWorkContext<VM> {
    type VM = VM;
    type PlanType = Compressor<VM>;
    type DefaultProcessEdges = MarkingProcessEdges<VM>;
    type PinningProcessEdges = UnsupportedProcessEdges<VM>;
}

pub struct CompressorForwardingWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for CompressorForwardingWorkContext<VM> {
    type VM = VM;
    type PlanType = Compressor<VM>;
    type DefaultProcessEdges = ForwardingProcessEdges<VM>;
    type PinningProcessEdges = UnsupportedProcessEdges<VM>;
}
