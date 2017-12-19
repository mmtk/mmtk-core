use super::space::default;

use std::sync::Mutex;

use ::policy::space::Space;
use ::util::heap::{PageResource, MonotonePageResource};
use ::util::address::Address;

use ::util::ObjectReference;

pub struct ImmortalSpace {
    pr: Mutex<MonotonePageResource>,
}

impl Space for ImmortalSpace {
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

impl ImmortalSpace {
    pub fn new() -> Self {
        ImmortalSpace {
            pr: Mutex::new(MonotonePageResource::new()),
        }
    }
}