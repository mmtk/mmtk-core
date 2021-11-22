use super::global::GenCopy;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::policy::copyspace::CopySpaceCopyContext;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use std::ops::{Deref, DerefMut};
use crate::util::copy::*;

pub struct GenCopyMatureProcessEdges<VM: VMBinding> {
    plan: &'static GenCopy<VM>,
    base: ProcessEdgesBase<GenCopyMatureProcessEdges<VM>>,
}

impl<VM: VMBinding> GenCopyMatureProcessEdges<VM> {
    fn gencopy(&self) -> &'static GenCopy<VM> {
        self.plan
    }
}

impl<VM: VMBinding> ProcessEdgesWork for GenCopyMatureProcessEdges<VM> {
    type VM = VM;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<GenCopy<VM>>().unwrap();
        Self { plan, base }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // Evacuate mature objects; don't trace objects if they are in to-space
        if self.gencopy().tospace().in_space(object) {
            return object;
        } else if self.gencopy().fromspace().in_space(object) {
            return self.gencopy().fromspace().trace_object::<Self>(
                self,
                object,
                CopySemantics::DefaultCopy,
                self.worker(),
            );
        }

        self.gencopy()
            .gen
            .trace_object_full_heap::<Self>(self, object, self.worker())
    }
}

impl<VM: VMBinding> Deref for GenCopyMatureProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for GenCopyMatureProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub struct GenCopyNurseryGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyNurseryGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type CopyContextType = CopySpaceCopyContext<VM>;
    type ProcessEdgesWorkType = GenNurseryProcessEdges<VM>;
}

pub(super) struct GenCopyMatureGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for GenCopyMatureGCWorkContext<VM> {
    type VM = VM;
    type PlanType = GenCopy<VM>;
    type CopyContextType = CopySpaceCopyContext<VM>;
    type ProcessEdgesWorkType = GenCopyMatureProcessEdges<VM>;
}
