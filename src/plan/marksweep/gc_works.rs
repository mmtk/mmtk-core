use super::global::MarkSweep;
use crate::{plan::CopyContext};
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::alloc::{Allocator, FreeListAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

pub struct MyCopyContext<VM: VMBinding> {
    plan: &'static MarkSweep<VM>,
    allocator: FreeListAllocator<VM>,
}

impl<VM:VMBinding> CopyContext for MyCopyContext<VM> {
    type VM = VM;
    fn new(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: &mmtk.plan,
            allocator: FreeListAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
        }
    }
    fn init(&mut self, tls: OpaquePointer) {
        self.allocator.tls = tls;
    }
    fn prepare(&mut self) {
        //self.allocator.rebind(Some(self.plan.tospace()));
    }
    fn release(&mut self) {
        // self.allocator.rebind(Some(self.plan.tospace()));
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
        self.allocator.alloc(bytes, align, offset)
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
pub struct MyProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<MyProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for MyProcessEdges<VM> {
    type VM = VM;
    const OVERWRITE_REFERENCE: bool = false;
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

        //if not marked, mark and call self.process_node
        let a = object.to_address() - 8;
        let marking_word: usize = unsafe { a.load() };
        if marking_word == 0 {
            unsafe { a.store(1usize)};
            self.process_node(object);
        }
        object
    }
}

impl<VM: VMBinding> Deref for MyProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MyProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}