use std::sync::Mutex;

use ::policy::space::Space;
use ::util::heap::{PageResource, MonotonePageResource};
use ::util::address::Address;

use ::vm::scheduler::block_for_gc;

use ::plan::selected_plan;

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

    fn acquire(&self, thread_id: usize, size: usize) -> Address {
        let ret: Address = self.pr.lock().unwrap().get_new_pages(size);

        if ret.is_zero() && cfg!(feature = "jikesrvm") {
            selected_plan::PLAN.control_collector_context.request();
            block_for_gc(thread_id);
        }

        ret
    }
}