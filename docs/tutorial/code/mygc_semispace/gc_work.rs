// ANCHOR: imports
use super::global::MyGC;
use crate::plan::CopyContext;
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::object_forwarding;
use crate::util::{Address, ObjectReference};
use crate::util::opaque_pointer::*;
use crate::vm::VMBinding;
use crate::MMTK;
use crate::plan::PlanConstraints;
use crate::scheduler::WorkerLocal;
use std::ops::{Deref, DerefMut};
// ANCHOR_END: imports

// ANCHOR: mygc_copy_context
pub struct MyGCCopyContext<VM: VMBinding> {
    plan:&'static MyGC<VM>,
    mygc: BumpAllocator<VM>,
}
// ANCHOR_END: mygc_copy_context

impl<VM: VMBinding> CopyContext for MyGCCopyContext<VM> {
    type VM = VM;

    // ANCHOR: copycontext_constraints_init
    fn constraints(&self) -> &'static PlanConstraints {
        &super::global::MYGC_CONSTRAINTS
    }
    fn init(&mut self, tls: VMWorkerThread) {
        self.mygc.tls = tls.0;
    }
    // ANCHOR_END: copycontext_constraints_init
    // ANCHOR: copycontext_prepare
    fn prepare(&mut self) {
        self.mygc.rebind(self.plan.tospace());
    }
    // ANCHOR_END: copycontext_prepare
    fn release(&mut self) {
    }
    // ANCHOR: copycontext_alloc_copy
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
    // ANCHOR_END: copycontext_alloc_copy
    // ANCHOR: copycontext_post_copy
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
    // ANCHOR_END: copycontext_post_copy
}

// ANCHOR: constructor_and_workerlocal
impl<VM: VMBinding> MyGCCopyContext<VM> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        let plan = &mmtk.plan.downcast_ref::<MyGC<VM>>().unwrap();
        Self {
            plan,
            mygc: BumpAllocator::new(VMThread::UNINITIALIZED, plan.tospace(), &*mmtk.plan),
        }
    }
}

impl<VM: VMBinding> WorkerLocal for MyGCCopyContext<VM> {
    fn init(&mut self, tls: VMWorkerThread) {
        CopyContext::init(self, tls);
    }
}
// ANCHOR_END: constructor_and_workerlocal

// ANCHOR: mygc_process_edges
pub struct MyGCProcessEdges<VM: VMBinding> {
    plan: &'static MyGC<VM>,
    base: ProcessEdgesBase<MyGCProcessEdges<VM>>,
}
// ANCHOR_END: mygc_process_edges

impl<VM: VMBinding> MyGCProcessEdges<VM> {
    fn mygc(&self) -> &'static MyGC<VM> {
        self.plan
    }
}

impl<VM:VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM> {
    type VM = VM;
    // ANCHOR: mygc_process_edges_new
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        let base = ProcessEdgesBase::new(edges, mmtk);
        let plan = base.plan().downcast_ref::<MyGC<VM>>().unwrap();
        Self { base, plan }
    }
    // ANCHOR_END: mygc_process_edges_new

    // ANCHOR: trace_object
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
    // ANCHOR_END: trace_object
}

// ANCHOR: deref
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
// ANCHOR_END: deref