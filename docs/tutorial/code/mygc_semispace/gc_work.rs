use super::global::MyGC;
use crate::plan::CopyContext;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use crate::MMTK;
use crate::plan::PlanConstraints;
use crate::scheduler::WorkerLocal;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub struct MyGCCopyContext<VM: VMBinding> {
    plan:&'static MyGC<VM>,
    mygc: BumpAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for MyGCCopyContext<VM> {
    type VM = VM;

    fn constraints(&self) -> &'static PlanConstraints {
        &super::global::MYGC_CONSTRAINTS
    }
    fn init(&mut self, tls:OpaquePointer) {
        self.mygc.tls = tls;
    }
    fn prepare(&mut self) {
        self.mygc.rebind(Some(self.plan.tospace()));
    }
    fn release(&mut self) {
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
        self.mygc.alloc(bytes, align, offset)
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

impl<VM: VMBinding> MyGCCopyContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            plan: &mmtk.plan.downcast_ref::<MyGC<VM>>().unwrap(),
            mygc: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &*mmtk.plan),
        }
    }
}

impl<VM: VMBinding> WorkerLocal for MyGCCopyContext<VM> {
    fn init(&mut self, tls: OpaquePointer) {
        CopyContext::init(self, tls);
    }
}

pub struct MyGCProcessEdges<VM: VMBinding> {
    plan: &'static MyGC<VM>,
    base: ProcessEdgesBase<MyGCProcessEdges<VM>>,
}

impl<VM: VMBinding> MyGCProcessEdges<VM> {
    fn mygc(&self) -> &'static MyGC<VM> {
        self.plan
    }
}

impl<VM:VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<MyGC<VM>>().unwrap();
        Self { base, plan }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.mygc().tospace().in_space(object) {
            self.mygc().tospace().trace_object::<Self, MyGCCopyContext<VM>>(
                self,
                object,
                super::global::ALLOC_MyGC,
                unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
            )
        } else if self.mygc().fromspace().in_space(object) {
            self.mygc().fromspace().trace_object::<Self, MyGCCopyContext<VM>>(
                self,
                object,
                super::global::ALLOC_MyGC,
                unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
            )
        } else {
            self.mygc().common.trace_object::<Self, MyGCCopyContext<VM>>(self, object)
        }
    }
}

impl<VM: VMBinding> Deref for MyGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MyGCProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
