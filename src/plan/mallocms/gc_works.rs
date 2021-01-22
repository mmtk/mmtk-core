use crate::{policy::malloc::{MARKING_METADATA_ID, is_marked, set_mark_bit}, scheduler::gc_works::*, util::side_metadata::SideMetadata};
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use std::{marker::PhantomData};
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
        if !is_marked(object) {
            set_mark_bit(object.to_address());
            self.process_node(object);
        }
        object
        
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