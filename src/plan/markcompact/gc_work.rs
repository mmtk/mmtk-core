use super::global::MarkCompact;
use crate::plan::global::NoCopy;
use crate::policy::markcompactspace::MarkCompactSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::util::{Address, ObjectReference};
use crate::vm::Scanning;
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

pub struct CalcFwdAddr<VM: VMBinding> {
    mc_space: &'static MarkCompactSpace<VM>,
}

impl<VM: VMBinding> GCWork<VM> for CalcFwdAddr<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.mc_space.calcluate_forwarding_pointer();
        // FIXME
        // The following needs to be done right before the second round of root scanning
        // put here for simplicity since calculating forwarding pointer occurs right
        // before updating object references(done through another round of root scanning)
        // and this calculation is done in a single-threaded manner.
        VM::VMScanning::prepare_for_roots_scanning();
    }
}

impl<VM: VMBinding> CalcFwdAddr<VM> {
    pub fn new(mc_space: &'static MarkCompactSpace<VM>) -> Self {
        Self { mc_space }
    }
}

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
