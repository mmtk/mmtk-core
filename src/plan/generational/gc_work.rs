use atomic::Ordering;

use crate::plan::PlanConstraints;
use crate::plan::{barriers::BarrierSelector, CopyContext};
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkerLocal;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::object_forwarding;
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use crate::plan::generational::global::Gen;
use std::ops::{Deref, DerefMut};

pub struct GenNurseryProcessEdges<VM: VMBinding, C: CopyContext + GCWorkerLocal> {
    gen: &'static Gen<VM>,
    base: ProcessEdgesBase<GenNurseryProcessEdges<VM, C>>,
}

// impl<VM: VMBinding> GenCopyNurseryProcessEdges<VM> {
//     fn gencopy(&self) -> &'static GenCopy<VM> {
//         self.plan
//     }
// }

impl<VM: VMBinding, C: CopyContext + GCWorkerLocal> ProcessEdgesWork for GenNurseryProcessEdges<VM, C> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let gen = base.plan().generational();
        Self { gen, base }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        self.gen.trace_object_nursery(self, object, unsafe { self.worker().local::<C>() })
        // // Evacuate nursery objects
        // if self.gencopy().gen.nursery.in_space(object) {
        //     return self
        //         .gencopy()
        //         .gen.nursery
        //         .trace_object::<Self, GenCopyCopyContext<VM>>(
        //             self,
        //             object,
        //             super::global::ALLOC_SS,
        //             unsafe { self.worker().local::<GenCopyCopyContext<VM>>() },
        //         );
        // }
        // // We may alloc large object into LOS as nursery objects. Trace them here.
        // if self.gencopy().gen.common.get_los().in_space(object) {
        //     return self
        //         .gencopy()
        //         .gen.common
        //         .get_los()
        //         .trace_object::<Self>(self, object);
        // }
        // debug_assert!(!self.gencopy().fromspace().in_space(object));
        // debug_assert!(self.gencopy().tospace().in_space(object));
        // object
    }
    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        debug_assert!(!self.gen.nursery.in_space(new_object));
        unsafe { slot.store(new_object) };
    }
}

impl<VM: VMBinding, C: CopyContext + GCWorkerLocal> Deref for GenNurseryProcessEdges<VM, C> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, C: CopyContext + GCWorkerLocal> DerefMut for GenNurseryProcessEdges<VM, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}