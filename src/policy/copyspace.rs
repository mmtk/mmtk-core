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
        let mut forwarding_word = ForwardingWord::attempt_to_forward(object);
        if ForwardingWord::state_is_forwarded_or_being_forwarded(forwarding_word) {
            while ForwardingWord::state_is_being_forwarded(forwarding_word) {
                forwarding_word = VMObjectModel::read_available_bits_word(object);
            }
            return ForwardingWord::extract_forwarding_pointer(forwarding_word);
        } else {
            let new_object = VMObjectModel::copy(object, allocator, thread_id);
            ForwardingWord::set_forwarding_pointer(object, new_object);
            trace.process_node(new_object);
            trace!("Copying [{:?} -> {:?}]", object, new_object);
            return new_object;
        }
    }
}