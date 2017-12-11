use std::sync::Mutex;

use ::policy::space::Space;
use ::util::heap::{PageResource, MonotonePageResource};
use ::util::address::Address;

pub struct ImmortalSpace {
    pr: Mutex<MonotonePageResource>,
}

impl Space for ImmortalSpace {
    fn new() -> Self {
        ImmortalSpace {
            pr: Mutex::new(MonotonePageResource::new()),
        }
    }

    fn init(&self, heap_size: usize) {
        self.pr.lock().unwrap().init(heap_size);
    }

    fn acquire(&self, size: usize) -> Address {
        self.pr.lock().unwrap().get_new_pages(size)
    }
}