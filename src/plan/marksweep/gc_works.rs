use crate::scheduler::gc_works::*;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use std::{marker::PhantomData, ops::Sub};
use std::ops::{Deref, DerefMut};
use crate::policy::malloc::*;

pub struct WriteMallocBits {

}

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
        // println!("trace");
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
            //using metadata table
            if !is_marked(object) { 
                let ref mut metadata_table = METADATA_TABLE.write().unwrap();
                let chunk_index = address_to_chunk_index_with_write(object.to_address(), metadata_table).unwrap();
                let ref mut row = metadata_table[chunk_index];
                let ref mut marked = row.2;
                let index = address_to_bitmap_index(object.to_address());
                marked[index] = 1;
                
                // MARKED.lock().unwrap().insert(object);
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