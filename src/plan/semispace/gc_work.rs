use super::global::SemiSpace;
use crate::policy::space::Space;
use crate::policy::copyspace::CopySpaceCopyContext;
use crate::scheduler::gc_work::*;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

pub struct SSProcessEdges<VM: VMBinding> {
    // Use a static ref to the specific plan to avoid overhead from dynamic dispatch or
    // downcast for each traced object.
    plan: &'static SemiSpace<VM>,
    base: ProcessEdgesBase<SSProcessEdges<VM>>,
}

impl<VM: VMBinding> SSProcessEdges<VM> {
    fn ss(&self) -> &'static SemiSpace<VM> {
        self.plan
    }
}

impl<VM: VMBinding> ProcessEdgesWork for SSProcessEdges<VM> {
    type VM = VM;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let plan = base.plan().downcast_ref::<SemiSpace<VM>>().unwrap();
        Self { plan, base }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }

        // We don't need to trace the object if it is already in the to-space
        if self.ss().tospace().in_space(object) {
            object
        } else if self.ss().fromspace().in_space(object) {
            self.ss()
                .fromspace()
                .trace_object::<Self>(
                    self,
                    object,
                    super::global::ALLOC_SS,
                    self.worker(),
                )
        } else {
            use crate::plan::global::NoCopy;
            self.ss()
                .common
                .trace_object::<Self, NoCopy<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for SSProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for SSProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub struct SSGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);
impl<VM: VMBinding> crate::scheduler::GCWorkContext for SSGCWorkContext<VM> {
    type VM = VM;
    type PlanType = SemiSpace<VM>;
    type CopyContextType = CopySpaceCopyContext<VM>;
    type ProcessEdgesWorkType = SSProcessEdges<VM>;
}
