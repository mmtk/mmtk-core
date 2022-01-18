use super::global::PageProtect;
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

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<PageProtect<VM>>().unwrap();
        Self { plan, base }
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
            self.plan.common.trace_object::<Self>(self, object)
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

pub struct PPGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for PPGCWorkContext<VM> {
    type VM = VM;
    type PlanType = PageProtect<VM>;
    type ProcessEdgesWorkType = PPProcessEdges<VM>;
}
