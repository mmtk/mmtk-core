use super::global::Page;
use crate::plan::global::NoCopy;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

pub struct PageProcessEdges<VM: VMBinding> {
    // Use a static ref to the specific plan to avoid overhead from dynamic dispatch or
    // downcast for each traced object.
    plan: &'static Page<VM>,
    base: ProcessEdgesBase<PageProcessEdges<VM>>,
}

impl<VM: VMBinding> ProcessEdgesWork for PageProcessEdges<VM> {
    const OVERWRITE_REFERENCE: bool = false;
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<Page<VM>>().unwrap();
        Self { base, plan }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        let object = unsafe {
            let untagged_word = object.to_address().as_usize() & !0b11usize;
            Address::from_usize(untagged_word).to_object_reference()
        };
        if object.is_null() {
            return object;
        }
        if self.plan.space.in_space(object) {
            self.plan.space.trace_object::<Self>(self, object)
        } else {
            self.plan.common.trace_object::<Self, NoCopy<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for PageProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for PageProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
