use super::global::MarkSweep;
use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::alloc::{Allocator, FreeListAllocator};
use crate::util::forwarding_word;
use crate::util::{Address, ObjectReference, OpaquePointer};
use crate::vm::VMBinding;
use crate::MMTK;
use std::{marker::PhantomData, ops::Sub};
use std::ops::{Deref, DerefMut};
use crate::policy::malloc::*;

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

        if USE_HASHSET {
            // using hashset
            // if not marked, mark and call self.process_node
            let mark_address: Address = object.to_address().sub(8);
            let marking_word: usize = unsafe { mark_address.load() };
            if marking_word == 0usize {
                unsafe { mark_address.store(1) };
                self.process_node(object);
            }
            object
        } else {
            //using bitmaps
            if !is_marked(object) {
                mark(object);
                self.process_node(object);
            }
            object
        }
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