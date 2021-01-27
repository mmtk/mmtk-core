use super::global::GenCopy;
use crate::plan::{CopyContext, Plan};
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::scheduler::{GCWork, GCWorker, WorkBucketStage};
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::*;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub struct GenCopyCopyContext<VM: VMBinding> {
    plan: &'static GenCopy<VM>,
    ss: BumpAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for GenCopyCopyContext<VM> {
    type VM = VM;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: unsafe { &*(&mmtk.plan as *const _ as *const GenCopy<VM>) },
            ss: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
        }
    }
    fn init(&mut self, tls: OpaquePointer) {
        self.ss.tls = tls;
    }
    fn prepare(&mut self) {
        self.ss.rebind(Some(self.plan.tospace()));
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
        forwarding_word::clear_forwarding_bits::<VM>(obj);
    }
}

#[derive(Default)]
pub struct GenCopyNurseryProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<GenCopyNurseryProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for GenCopyNurseryProcessEdges<VM> {
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
        // Evacuate nursery objects
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(
                self,
                object,
                super::global::ALLOC_SS,
                self.worker().local(),
            );
        }
        debug_assert!(!self.plan().fromspace().in_space(object));
        debug_assert!(self.plan().tospace().in_space(object));
        object
    }
    #[inline]
    fn process_edge(&mut self, slot: Address) {
        debug_assert!(!self.plan().fromspace().address_in_space(slot));
        let object = unsafe { slot.load::<ObjectReference>() };
        let new_object = self.trace_object(object);
        debug_assert!(!self.plan().nursery.in_space(new_object));
        unsafe { slot.store(new_object) };
    }
}

impl<VM: VMBinding> Deref for GenCopyNurseryProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for GenCopyNurseryProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

#[derive(Default)]
pub struct GenCopyMatureProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<GenCopyMatureProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for GenCopyMatureProcessEdges<VM> {
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
        // Evacuate nursery objects
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(
                self,
                object,
                super::global::ALLOC_SS,
                self.worker().local(),
            );
        }
        // Evacuate mature objects
        if self.plan().tospace().in_space(object) {
            return self.plan().tospace().trace_object(
                self,
                object,
                super::global::ALLOC_SS,
                self.worker().local(),
            );
        }
        if self.plan().fromspace().in_space(object) {
            return self.plan().fromspace().trace_object(
                self,
                object,
                super::global::ALLOC_SS,
                self.worker().local(),
            );
        }
        self.plan().common.trace_object(self, object)
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

#[derive(Default)]
pub struct GenCopyProcessModBuf {
    pub modified_nodes: Vec<ObjectReference>,
    pub modified_edges: Vec<Address>,
}

impl<VM: VMBinding> GCWork<VM> for GenCopyProcessModBuf {
    #[inline]
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        if mmtk.plan.in_nursery() {
            let mut modified_nodes = vec![];
            ::std::mem::swap(&mut modified_nodes, &mut self.modified_nodes);
            let work = ScanObjects::<GenCopyNurseryProcessEdges<VM>>::new(modified_nodes, false);
            worker.scheduler().work_buckets[WorkBucketStage::Closure].add(work);

            let mut modified_edges = vec![];
            ::std::mem::swap(&mut modified_edges, &mut self.modified_edges);
            worker.scheduler().work_buckets[WorkBucketStage::Closure]
                .add(GenCopyNurseryProcessEdges::<VM>::new(modified_edges, true));
        } else {
            // Do nothing
        }
    }
}
