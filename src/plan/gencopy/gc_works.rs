use super::global::GenCopy;
use crate::scheduler::gc_works::*;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use crate::policy::space::Space;
use crate::util::alloc::{BumpAllocator, Allocator};
use crate::util::forwarding_word;
use crate::MMTK;
use crate::plan::CopyContext;

pub struct GenCopyCopyContext<VM: VMBinding> {
    plan: &'static GenCopy<VM>,
    ss: BumpAllocator<VM>,
}

impl <VM: VMBinding> CopyContext for GenCopyCopyContext<VM> {
    type VM = VM;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: unsafe { &*(&mmtk.plan as *const _ as *const GenCopy<VM>) },
            ss: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
        }
    }
    fn prepare(&mut self) {
        self.ss.rebind(Some(self.plan.tospace()));
    }
    fn release(&mut self) {
        // self.ss.rebind(Some(self.plan.tospace()));
    }
    #[inline(always)]
    fn alloc_copy(&mut self, _original: ObjectReference, bytes: usize, align: usize, offset: isize, _allocator: crate::Allocator) -> Address {
        self.ss.alloc(bytes, align, offset)
    }
    #[inline(always)]
    fn post_copy(&mut self, obj: ObjectReference, _tib: Address, _bytes: usize, _allocator: crate::Allocator) {
        forwarding_word::clear_forwarding_bits::<VM>(obj);
    }
}




#[derive(Default)]
pub struct GenCopyNurseryProcessEdges<VM: VMBinding>  {
    base: ProcessEdgesBase<GenCopyNurseryProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl <VM: VMBinding> ProcessEdgesWork for GenCopyNurseryProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self { base: ProcessEdgesBase::new(edges), ..Default::default() }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // Evacuate nursery objects
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        // Mark mature objects
        if self.plan().tospace().in_space(object) {
            return self.plan().tospace().trace_mark_object(self, object);
        }
        if self.plan().fromspace().in_space(object) {
            return self.plan().fromspace().trace_mark_object(self, object);
        }
        self.plan().common.trace_object(self, object)
    }
}

impl <VM: VMBinding> Deref for GenCopyNurseryProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl <VM: VMBinding> DerefMut for GenCopyNurseryProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}



#[derive(Default)]
pub struct GenCopyMatureProcessEdges<VM: VMBinding>  {
    base: ProcessEdgesBase<GenCopyMatureProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl <VM: VMBinding> ProcessEdgesWork for GenCopyMatureProcessEdges<VM> {
    type VM = VM;
    fn new(edges: Vec<Address>, _roots: bool) -> Self {
        Self { base: ProcessEdgesBase::new(edges), ..Default::default() }
    }
    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        // Evacuate nursery objects
        if self.plan().nursery.in_space(object) {
            return self.plan().nursery.trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        // Evacuate mature objects
        if self.plan().tospace().in_space(object) {
            return self.plan().tospace().trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        if self.plan().fromspace().in_space(object) {
            return self.plan().fromspace().trace_object(self, object, super::global::ALLOC_SS, self.worker().local());
        }
        self.plan().common.trace_object(self, object)
    }
}

impl <VM: VMBinding> Deref for GenCopyMatureProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl <VM: VMBinding> DerefMut for GenCopyMatureProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}