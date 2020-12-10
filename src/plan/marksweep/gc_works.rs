use super::global::MarkSweep;
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::alloc::{Allocator, FreeListAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

#[derive(Default)]
pub struct MSProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<MSProcessEdges<VM>>,
    phantom: PhantomData<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for MSProcessEdges<VM> {
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
        if marking_word == 0usize {
            unsafe { a.store(1) };
            self.process_node(object);
        }
        object
            //     unsafe { a.store(mark_count_u8)};
            //     self.process_node(object);


        // let marking_word: usize = unsafe { a.load() };
        // let mark_count_u8 = plan;
        // if marking_word != mark_count_u8 as usize {
        //     unsafe { a.store(mark_count_u8)};
        //     self.process_node(object);
        // }
        // object
    }
}

impl<VM: VMBinding> Deref for MSProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for MSProcessEdges<VM> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}