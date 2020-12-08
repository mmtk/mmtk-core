use super::global::MyGenCopy;
use crate::{Plan, plan::CopyContext, scheduler::{GCWork, GCWorker}};
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub struct MGCCopyContext<VM: VMBinding> {
    plan: &'static MyGenCopy<VM>,
    mgc: BumpAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for MGCCopyContext<VM> {
    type VM = VM;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: unsafe { &*(&mmtk.plan as *const _ as *const MyGenCopy<VM>) },
            mgc: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
        }
    }
    fn init(&mut self, tls: OpaquePointer) {
        self.mgc.tls = tls;
    }
    fn prepare(&mut self) {
        self.mgc.rebind(Some(&self.plan.mature));
    }
    fn release(&mut self) {
        // Do nothing
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
        self.mgc.alloc(bytes, align, offset)
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

#[derive(Default)]
pub struct MGCNurseryProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<MGCNurseryProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for MGCNurseryProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self {
            base: ProcessEdgesBase::new(edges),
            ..Default::default()
        }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(
                self,
                object,
                super::global::ALLOC_MGC,
                self.worker().local(),
            )
        }
        object
    }

    #[inline]
    fn process_edge(&mut self, slot: Address) {
        let object = unsafe {slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        unsafe { slot.store(new_object) };
    }
}

impl<VM: VMBinding> Deref for MGCNurseryProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
impl<VM: VMBinding> DerefMut for MGCNurseryProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}


#[derive(Default)]
pub struct MGCMatureProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<MGCMatureProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for MGCMatureProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self {
            base: ProcessEdgesBase::new(edges),
            ..Default::default()
        }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(
                self,
                object,
                super::global::ALLOC_MGC,
                self.worker().local(),
            )
        }
        if self.plan().mature.in_space(object) {
            return self.plan().mature.trace_object(
                self,
                object,
            )
        }
        self.plan().common.trace_object(self,object)
    }
}

impl<VM: VMBinding> Deref for MGCMatureProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
impl<VM: VMBinding> DerefMut for MGCMatureProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}


#[derive(Default)]
pub struct MyGenCopyProcessModBuf {
    pub modified_nodes: Vec<ObjectReference>,
    pub modified_edges: Vec<Address>,
}

impl<VM: VMBinding> GCWork<VM> for MyGenCopyProcessModBuf {
    #[inline]
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        if mmtk.plan.in_nursery() {
            let mut modified_nodes = vec![];
            ::std::mem::swap(&mut modified_nodes, &mut self.modified_nodes);
            worker.scheduler().closure_stage.add(
                ScanObjects::<MGCNurseryProcessEdges<VM>>::new(modified_nodes, false),
            );

            let mut modified_edges = vec![];
            ::std::mem::swap(&mut modified_edges, &mut self.modified_edges);
            worker
                .scheduler()
                .closure_stage
                .add(MGCNurseryProcessEdges::<VM>::new(modified_edges, true));
        } else {
            // Do nothing
        }
    }
}
