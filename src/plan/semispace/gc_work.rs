use super::global::SemiSpace;
use crate::plan::CopyContext;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::GCWorkerLocal;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::object_forwarding;
use crate::util::opaque_pointer::*;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

pub struct SSCopyContext<VM: VMBinding> {
    plan: &'static SemiSpace<VM>,
    ss: BumpAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for SSCopyContext<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &super::global::SS_CONSTRAINTS
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
    }
}

impl<VM: VMBinding> SSCopyContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        let plan = &mmtk.plan.downcast_ref::<SemiSpace<VM>>().unwrap();
        Self {
            plan,
            // it doesn't matter which space we bind with the copy allocator. We will rebind to a proper space in prepare().
            ss: BumpAllocator::new(VMThread::UNINITIALIZED, plan.tospace(), &*mmtk.plan),
        }
    }
}

impl<VM: VMBinding> GCWorkerLocal for SSCopyContext<VM> {
    fn init(&mut self, tls: VMWorkerThread) {
        CopyContext::init(self, tls);
    }
}

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
                .trace_object::<Self, SSCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_SS,
                    unsafe { self.worker().local::<SSCopyContext<VM>>() },
                )
        } else {
            self.ss()
                .common
                .trace_object::<Self, SSCopyContext<VM>>(self, object)
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
