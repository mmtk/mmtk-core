use super::global::Immix;
use crate::plan::CopyContext;
use crate::plan::PlanConstraints;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::WorkerLocal;
use crate::util::alloc::{Allocator, ImmixAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use crate::MMTK;
use std::ops::{Deref, DerefMut};

pub struct ImmixCopyContext<VM: VMBinding> {
    plan: &'static Immix<VM>,
    immix: ImmixAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for ImmixCopyContext<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &super::global::IMMIX_CONSTRAINTS
    }
    fn init(&mut self, tls: OpaquePointer) {
        self.immix.tls = tls;
    }
    fn prepare(&mut self) {
        self.immix.rebind(Some(self.plan.tospace()));
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
        self.immix.alloc(bytes, align, offset)
    }
    #[inline(always)]
    fn post_copy(
        &mut self,
        obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        _semantics: crate::AllocationSemantics,
    ) {
        forwarding_word::clear_forwarding_bits::<VM>(obj);
    }
}

impl<VM: VMBinding> ImmixCopyContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            plan: &mmtk.plan.downcast_ref::<Immix<VM>>().unwrap(),
            immix: ImmixAllocator::new(OpaquePointer::UNINITIALIZED, None, &*mmtk.plan),
        }
    }
}

impl<VM: VMBinding> WorkerLocal for ImmixCopyContext<VM> {
    fn init(&mut self, tls: OpaquePointer) {
        CopyContext::init(self, tls);
    }
}

pub struct ImmixProcessEdges<VM: VMBinding> {
    // Use a static ref to the specific plan to avoid overhead from dynamic dispatch or
    // downcast for each traced object.
    plan: &'static Immix<VM>,
    base: ProcessEdgesBase<ImmixProcessEdges<VM>>,
}

impl<VM: VMBinding> ImmixProcessEdges<VM> {
    fn immix(&self) -> &'static Immix<VM> {
        self.plan
    }
}

impl<VM: VMBinding> ProcessEdgesWork for ImmixProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<Immix<VM>>().unwrap();
        Self { base, plan }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.immix().tospace().in_space(object) {
            self.immix().tospace().trace_object::<Self, ImmixCopyContext<VM>>(
                self,
                object,
                super::global::ALLOC_IMMIX,
                unsafe { self.worker().local::<ImmixCopyContext<VM>>() },
            )
        } else if self.immix().fromspace().in_space(object) {
            self.immix()
                .fromspace()
                .trace_object::<Self, ImmixCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_IMMIX,
                    unsafe { self.worker().local::<ImmixCopyContext<VM>>() },
                )
        } else {
            self.immix()
                .common
                .trace_object::<Self, ImmixCopyContext<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for ImmixProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for ImmixProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
