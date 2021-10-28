use super::global::GenCopy;
use crate::plan::generational::gc_work::GenNurseryProcessEdges;
use crate::plan::CopyContext;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkerLocal;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
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
        tib: Address,
        bytes: usize,
        semantics: crate::AllocationSemantics,
    ) {
        crate::plan::generational::generational_post_copy::<VM>(obj, tib, bytes, semantics)
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
            .trace_object_full_heap::<Self, GenCopyCopyContext<VM>>(self, object, unsafe {
                self.worker().local::<GenCopyCopyContext<VM>>()
            })
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

pub struct GenCopyNurseryGCWorkContext;
impl<VM: VMBinding> crate::scheduler::GCWorkContext<VM> for GenCopyNurseryGCWorkContext {
    type PlanType = GenCopy<VM>;
    type CopyContextType = GenCopyCopyContext<VM>;
    type ProcessEdgesWorkType = GenNurseryProcessEdges<VM, Self::CopyContextType>;
}

pub(super) struct GenCopyMatureGCWorkContext;
impl<VM: VMBinding> crate::scheduler::GCWorkContext<VM> for GenCopyMatureGCWorkContext {
    type PlanType = GenCopy<VM>;
    type CopyContextType = GenCopyCopyContext<VM>;
    type ProcessEdgesWorkType = GenCopyMatureProcessEdges<VM>;
}
