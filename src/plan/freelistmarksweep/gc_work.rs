use crate::plan::global::NoCopy;
use crate::plan::global::Plan;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

use super::FreeListMarkSweep;

pub struct FLMSProcessEdges<VM: VMBinding> {
    plan: &'static FreeListMarkSweep<VM>,
    base: ProcessEdgesBase<FLMSProcessEdges<VM>>,
}

impl<VM: VMBinding> ProcessEdgesWork for FLMSProcessEdges<VM> {
    type VM = VM;
    const OVERWRITE_REFERENCE: bool = false;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<FreeListMarkSweep<VM>>().unwrap();
        Self { plan, base }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        trace!("Tracing object {}", object);
        if self.plan.ms_space.in_space(object) {
            self.plan.ms_space.trace_object::<Self>(self, object)
        } else {
            self.plan.im_space.trace_object::<Self>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for FLMSProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for FLMSProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
