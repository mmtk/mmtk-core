
use atomic::Ordering;

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
        // unreachable!();
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
            if !MARKED.lock().unwrap().contains(&object) { 
                // !is_marked(object) {
                // assert!(!MARKED.lock().unwrap().contains(&object));
                // println!("setting mark bit for obj {}", object.to_address().as_usize());
                MARKED.lock().unwrap().insert(object);
                let buffer_full = {
                    let mut mark_buffer = MARK_BUFFER.lock().unwrap();
                    mark_buffer.push((object.to_address(),1));
                    mark_buffer.len() >= 16
                };
                if buffer_full {
                    write_mark_bits();
                }
                // unsafe { MARK_BUFFER.lock().unwrap().push(vec![(object.to_address(),1)]) };
                // set_mark_bit(object);
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