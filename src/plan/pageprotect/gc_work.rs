use super::global::PageProtect;
use crate::plan::global::NoCopy;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

/// Edge scanning work packet.
pub struct PPProcessEdges<VM: VMBinding> {
    /// Use a static ref to the specific plan to avoid overhead from dynamic dispatch or
    /// downcast for each traced object.
    plan: &'static PageProtect<VM>,
    base: ProcessEdgesBase<PPProcessEdges<VM>>,
}

impl<VM: VMBinding> ProcessEdgesWork for PPProcessEdges<VM> {
    const OVERWRITE_REFERENCE: bool = false;
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<PageProtect<VM>>().unwrap();
        Self { plan, base }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan.space.in_space(object) {
            self.plan.space.trace_object::<Self>(self, object)
        } else {
            self.plan
                .common
                .trace_object::<Self, NoCopy<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for PPProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for PPProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
