use super::global::SemiSpace;
use crate::plan::CopyContext;
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::alloc::{Allocator, BumpAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub struct SSCopyContext<VM: VMBinding> {
    plan: &'static SemiSpace<VM>,
    ss: BumpAllocator<VM>,
}

impl<VM: VMBinding> CopyContext for SSCopyContext<VM> {
    type VM = VM;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: &mmtk.plan,
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
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
        _allocator: crate::Allocator,
    ) -> Address {
        self.ss.alloc(bytes, align, offset)
    }
    #[inline(always)]
    fn post_copy(
        &mut self,
        obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        _allocator: crate::Allocator,
    ) {
        forwarding_word::clear_forwarding_bits::<VM>(obj);
    }
}

#[derive(Default)]
pub struct SSProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<SSProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for SSProcessEdges<VM> {
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
        let from_space = self.plan().fromspace();
        if from_space.in_space(object) {
            return from_space.trace_object(
                self,
                object,
                super::global::ALLOC_SS,
                self.worker().local(),
            );
        }
        object
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
