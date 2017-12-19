use super::space::default;

use std::sync::Mutex;

use ::util::heap::PageResource;
use ::util::heap::MonotonePageResource;

use ::policy::space::Space;
use ::util::{Address, ObjectReference};

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
}