// ANCHOR: imports
use super::global::MyGC;
use crate::scheduler::{gc_work::*, WorkBucketStage};
use crate::vm::VMBinding;
use std::ops::{Deref, DerefMut};
// ANCHOR_END: imports

// ANCHOR: workcontext_sft
pub struct MyGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type ProcessEdgesWorkType = SFTProcessEdges<Self::VM>;
    type TPProcessEdges = UnsupportedProcessEdges<Self::VM>;
}
// ANCHOR_END: workcontext_sft

// ANCHOR: workcontext_plan
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::scheduler::gc_work::PlanProcessEdges;
pub struct MyGCWorkContext2<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext2<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type ProcessEdgesWorkType = PlanProcessEdges<Self::VM, MyGC<VM>, DEFAULT_TRACE>;
    type TPProcessEdges = UnsupportedProcessEdges<Self::VM>;
}
// ANCHOR_END: workcontext_plan

use crate::policy::space::Space;
use crate::util::copy::CopySemantics;
use crate::util::ObjectReference;
use crate::MMTK;

// ANCHOR: mygc_process_edges
pub struct MyGCProcessEdges<VM: VMBinding> {
    plan: &'static MyGC<VM>,
    base: ProcessEdgesBase<VM>,
}
// ANCHOR_END: mygc_process_edges

// ANCHOR: mygc_process_edges_impl
impl<VM: VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM> {
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;

    fn new(
        edges: Vec<EdgeOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk, bucket);
        let plan = base.plan().downcast_ref::<MyGC<VM>>().unwrap();
        Self { base, plan }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let worker = self.worker();
        let queue = &mut self.base.nodes;
        if self.plan.tospace().in_space(object) {
            self.plan.tospace().trace_object(
                queue,
                object,
                Some(CopySemantics::DefaultCopy),
                worker,
            )
        } else if self.plan.fromspace().in_space(object) {
            self.plan.fromspace().trace_object(
                queue,
                object,
                Some(CopySemantics::DefaultCopy),
                worker,
            )
        } else {
            self.plan.common.trace_object(queue, object, worker)
        }
    }

    fn create_scan_work(&self, nodes: Vec<ObjectReference>, roots: bool) -> ScanObjects<Self> {
        ScanObjects::<Self>::new(nodes, false, roots, self.bucket)
    }
}
// ANCHOR_END: mygc_process_edges_impl

// ANCHOR: mygc_process_edges_deref
impl<VM: VMBinding> Deref for MyGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MyGCProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
// ANCHOR_END: mygc_process_edges_deref

// ANCHOR: workcontext_mygc
pub struct MyGCWorkContext3<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for MyGCWorkContext3<VM> {
    type VM = VM;
    type PlanType = MyGC<VM>;
    type ProcessEdgesWorkType = MyGCProcessEdges<Self::VM>;
    type TPProcessEdges = UnsupportedProcessEdges<Self::VM>;
}
// ANCHOR: workcontext_mygc
