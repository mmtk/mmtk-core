use super::global::MarkCompact;
use crate::policy::markcompactspace::MarkCompactSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::scheduler::WorkBucketStage;
use crate::util::{Address, ObjectReference};
use crate::vm::ActivePlan;
use crate::vm::Scanning;
use crate::vm::VMBinding;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

/// iterate through the heap and calculate the new location of live objects
pub struct CalculateForwardingAddress<VM: VMBinding> {
    mc_space: &'static MarkCompactSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for CalculateForwardingAddress<VM> {
    #[inline]
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
    p: PhantomData<VM>,
}

impl<VM: VMBinding> GCWork<VM> for UpdateReferences<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        // The following needs to be done right before the second round of root scanning
        VM::VMScanning::prepare_for_roots_re_scanning();
        mmtk.plan.base().prepare_for_stack_scanning();
        #[cfg(feature = "extreme_assertions")]
        crate::util::edge_logger::reset();

        // Prepare the plan again and get ready for a second trace. This needs to be done
        // before any work for the second trace starts.
        {
            use crate::plan::global::Plan;
            let plan: &MarkCompact<VM> = mmtk.plan.downcast_ref::<MarkCompact<VM>>().unwrap();
            // This should be the only packet that is executing at the point.
            #[allow(clippy::cast_ref_to_mut)]
            let plan_mut: &mut MarkCompact<VM> = unsafe { &mut *(plan as *const _ as *mut _) };
            plan_mut.prepare(_worker.tls);
        }

        // The following will push work for the second trace.

        // TODO investigate why the following will create duplicate edges
        // scheduler.work_buckets[WorkBucketStage::RefForwarding]
        //     .add(ScanStackRoots::<ForwardingProcessEdges<VM>>::new());
        for mutator in VM::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::RefForwarding]
                .add(ScanStackRoot::<ForwardingProcessEdges<VM>>(mutator));
        }

        mmtk.scheduler.work_buckets[WorkBucketStage::RefForwarding]
            .add(ScanVMSpecificRoots::<ForwardingProcessEdges<VM>>::new());
    }
}

impl<VM: VMBinding> UpdateReferences<VM> {
    pub fn new() -> Self {
        Self { p: PhantomData }
    }
}

/// compact live objects based on forwarding pointers calculated before
pub struct Compact<VM: VMBinding> {
    mc_space: &'static MarkCompactSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for Compact<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.mc_space.compact();
    }
}

impl<VM: VMBinding> Compact<VM> {
    pub fn new(mc_space: &'static MarkCompactSpace<VM>) -> Self {
        Self { mc_space }
    }
}

// Transitive closure to mark live objects
pub struct MarkingProcessEdges<VM: VMBinding> {
    plan: &'static MarkCompact<VM>,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding> MarkingProcessEdges<VM> {
    fn markcompact(&self) -> &'static MarkCompact<VM> {
        self.plan
    }
}

impl<VM: VMBinding> ProcessEdgesWork for MarkingProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<MarkCompact<VM>>().unwrap();
        Self { base, plan }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }

        // record that we have traced this object in trace1
        #[cfg(debug_assertions)]
        self.markcompact().trace1.lock().unwrap().insert(object);

        if self.markcompact().mc_space().in_space(object) {
            self.markcompact()
                .mc_space()
                .trace_mark_object::<Self>(self, object)
        } else {
            self.markcompact().common.trace_object::<Self>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for MarkingProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MarkingProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

/// Transitive closure to update object references
pub struct ForwardingProcessEdges<VM: VMBinding> {
    plan: &'static MarkCompact<VM>,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding> ForwardingProcessEdges<VM> {
    fn markcompact(&self) -> &'static MarkCompact<VM> {
        self.plan
    }
}

impl<VM: VMBinding> ProcessEdgesWork for ForwardingProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<MarkCompact<VM>>().unwrap();
        Self { base, plan }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }

        // record that we have traced this object in trace2
        #[cfg(debug_assertions)]
        self.markcompact().trace2.lock().unwrap().insert(object);

        if self.markcompact().mc_space().in_space(object) {
            self.markcompact()
                .mc_space()
                .trace_forward_object::<Self>(self, object)
        } else {
            self.markcompact().common.trace_object::<Self>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for ForwardingProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for ForwardingProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub struct MarkCompactGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MarkCompactGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MarkCompact<VM>;
    type ProcessEdgesWorkType = MarkingProcessEdges<VM>;
}
