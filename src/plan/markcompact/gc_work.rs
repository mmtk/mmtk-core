use super::global::MarkCompact;
use crate::plan::global::NoCopy;
use crate::plan::CopyContext;
use crate::plan::PlanConstraints;
use crate::policy::markcompactspace::MarkCompactSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::scheduler::GCWorkerLocal;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

pub struct CalcFwdAddr<VM: VMBinding> {
    mc_space: &'static MarkCompactSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for CalcFwdAddr<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        // calcluate the forwarding pointer
        self.mc_space.calcluate_forwarding_pointer();
    }
}

impl<VM: VMBinding> CalcFwdAddr<VM> {
    pub fn new(mc_space: &'static MarkCompactSpace<VM>) -> Self {
        Self { mc_space }
    }
}

pub struct Info<VM: VMBinding> {
    mc_space: &'static MarkCompactSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for Info<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        // calcluate the forwarding pointer
        self.mc_space.info();
    }
}

impl<VM: VMBinding> Info<VM> {
    pub fn new(mc_space: &'static MarkCompactSpace<VM>) -> Self {
        Self { mc_space }
    }
}

pub struct MarkingProcessEdges<VM: VMBinding> {
    plan: &'static MarkCompact<VM>,
    base: ProcessEdgesBase<MarkingProcessEdges<VM>>,
}

impl<VM: VMBinding> MarkingProcessEdges<VM> {
    fn markcompact(&self) -> &'static MarkCompact<VM> {
        self.plan
    }
}

impl<VM: VMBinding> ProcessEdgesWork for MarkingProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<MarkCompact<VM>>().unwrap();
        Self { base, plan }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.markcompact().mc_space().in_space(object) {
            self.markcompact()
                .mc_space()
                .trace_mark_object::<Self>(self, object)
        } else {
            self.markcompact()
                .common
                .trace_object::<Self, NoCopy<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for MarkingProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
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

pub struct ForwardingProcessEdges<VM: VMBinding> {
    plan: &'static MarkCompact<VM>,
    base: ProcessEdgesBase<ForwardingProcessEdges<VM>>,
}

impl<VM: VMBinding> ForwardingProcessEdges<VM> {
    fn markcompact(&self) -> &'static MarkCompact<VM> {
        self.plan
    }
}

impl<VM: VMBinding> ProcessEdgesWork for ForwardingProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<MarkCompact<VM>>().unwrap();
        Self { base, plan }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.markcompact().mc_space().in_space(object) {
            self.markcompact()
                .mc_space()
                .trace_forward_object::<Self>(self, object)
        } else {
            self.markcompact()
                .common
                .trace_object::<Self, NoCopy<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for ForwardingProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
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
