use super::space::default;

use std::sync::Mutex;

use ::util::heap::PageResource;
use ::util::heap::MonotonePageResource;

use ::policy::space::Space;
use ::util::{Address, ObjectReference};
use ::plan::TransitiveClosure;
use ::util::forwarding_word as ForwardingWord;
use ::vm::ObjectModel;
use ::vm::VMObjectModel;
use ::plan::Allocator;

pub struct CopySpace {
    pr: Mutex<MonotonePageResource>,
    from_space: bool,
}

impl Space for CopySpace {
    fn init(&self, heap_size: usize) {
        default::init(&self.pr, heap_size);
    }

    fn acquire(&self, thread_id: usize, size: usize) -> Address {
        default::acquire(&self.pr, thread_id, size)
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        default::in_space(&self.pr, object)
    }
}

impl CopySpace {
    pub fn new(from_space: bool) -> Self {
        CopySpace {
            pr: Mutex::new(MonotonePageResource::new()),
            from_space,
        }
    }

    pub fn prepare(&mut self, from_space: bool) {
        self.from_space = from_space;
    }

    pub fn release(&self) {
        unimplemented!()
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        trace: &mut T,
        object: ObjectReference,
        allocator: Allocator,
        thread_id: usize,
    ) -> ObjectReference
    {
        trace!("copyspace.trace_object(, {:?}, {:?}, {:?})", object, allocator, thread_id);
        if !self.from_space {
            return object;
        }
        trace!("attempting to forward");
        let mut forwarding_word = ForwardingWord::attempt_to_forward(object);
        trace!("checking if object is being forwarded");
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_word) {
            trace!("... yes it is");
            while ForwardingWord::state_is_being_forwarded(forwarding_word) {
                forwarding_word = VMObjectModel::read_available_bits_word(object);
            }
            trace!("Returning");
            return ForwardingWord::extract_forwarding_pointer(forwarding_word);
        } else {
            trace!("... no it isn't. Copying");
            let new_object = VMObjectModel::copy(object, allocator, thread_id);
            trace!("Setting forwarding pointer");
            ForwardingWord::set_forwarding_pointer(object, new_object);
            trace!("Forwarding pointer");
            trace.process_node(new_object);
            trace!("Copying [{:?} -> {:?}]", object, new_object);
            return new_object;
        }
    }
}