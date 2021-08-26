use atomic::Ordering;

use super::global::GenCopy;
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

pub struct GenCopyCopyContext<VM: VMBinding> {
    plan: &'static GenCopy<VM>,
    ss: BumpAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for GenCopyCopyContext<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &super::global::GENCOPY_CONSTRAINTS
    }
    fn init(&mut self, tls: VMWorkerThread) {
        self.ss.tls = tls.0;
    }
    fn prepare(&mut self) {
        self.ss.rebind(self.plan.tospace());
    }
    fn release(&mut self) {
        // self.ss.rebind(Some(self.plan.tospace()));
    }
    #[inline(always)]
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
        _semantics: crate::AllocationSemantics,
    ) -> Address {
        debug_assert!(VM::VMActivePlan::global().base().gc_in_progress_proper());
        self.ss.alloc(bytes, align, offset)
    }
    #[inline(always)]
    fn post_copy(
        &mut self,
        obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        _semantics: crate::AllocationSemantics,
    ) {
        object_forwarding::clear_forwarding_bits::<VM>(obj);
        if !super::NO_SLOW && super::ACTIVE_BARRIER == BarrierSelector::ObjectBarrier {
            VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC.mark_as_unlogged::<VM>(obj, Ordering::SeqCst);
        }
    }
}

impl<VM: VMBinding> GenCopyCopyContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        let plan = &mmtk.plan.downcast_ref::<GenCopy<VM>>().unwrap();
        Self {
            plan,
            // it doesn't matter which space we bind with the copy allocator. We will rebind to a proper space in prepare().
            ss: BumpAllocator::new(VMThread::UNINITIALIZED, plan.tospace(), &*mmtk.plan),
        }
    }
}

impl<VM: VMBinding> GCWorkerLocal for GenCopyCopyContext<VM> {
    fn init(&mut self, tls: VMWorkerThread) {
        CopyContext::init(self, tls);
    }
}

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
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<GenCopy<VM>>().unwrap();
        Self { plan, base }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // Evacuate nursery objects
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
        // Evacuate mature objects
        if self.gencopy().tospace().in_space(object) {
            return self
                .gencopy()
                .tospace()
                .trace_object::<Self, GenCopyCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_SS,
                    unsafe { self.worker().local::<GenCopyCopyContext<VM>>() },
                );
        }
        if self.gencopy().fromspace().in_space(object) {
            return self
                .gencopy()
                .fromspace()
                .trace_object::<Self, GenCopyCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_SS,
                    unsafe { self.worker().local::<GenCopyCopyContext<VM>>() },
                );
        }
        self.gencopy()
            .gen
            .trace_object_full_heap::<Self, GenCopyCopyContext<VM>>(self, object, unsafe { self.worker().local::<GenCopyCopyContext<VM>>() })
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
