use crate::plan::generational::global::Gen;
use crate::plan::CopyContext;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkerLocal;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

/// Process edges for a nursery GC. A generatinoal plan should use this type for a nursery GC.
pub struct GenNurseryProcessEdges<VM: VMBinding, C: CopyContext<VM = VM> + GCWorkerLocal> {
    gen: &'static Gen<VM>,
    base: ProcessEdgesBase<GenNurseryProcessEdges<VM, C>>,
}

impl<VM: VMBinding, C: CopyContext<VM = VM> + GCWorkerLocal> ProcessEdgesWork
    for GenNurseryProcessEdges<VM, C>
{
    type VM = VM;

    fn new(edges: Vec<Address>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk);
        let gen = base.plan().generational();
        Self { gen, base }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        self.gen
            .trace_object_nursery(self, object, self.worker())
    }
    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        debug_assert!(!self.gen.nursery.in_space(new_object));
        unsafe { slot.store(new_object) };
    }
}

impl<VM: VMBinding, C: CopyContext<VM = VM> + GCWorkerLocal> Deref
    for GenNurseryProcessEdges<VM, C>
{
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, C: CopyContext<VM = VM> + GCWorkerLocal> DerefMut
    for GenNurseryProcessEdges<VM, C>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
