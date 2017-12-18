use std::sync::Mutex;

use ::policy::space::Space;
use ::util::heap::{PageResource, MonotonePageResource};
use ::util::address::Address;

use ::vm::scheduler::block_for_gc;

use ::plan::selected_plan;
use ::util::ObjectReference;

pub struct ImmortalSpace {
    pr: Mutex<MonotonePageResource>,
}

impl Space for ImmortalSpace {
    fn init(&self, heap_size: usize) {
        self.pr.lock().unwrap().init(heap_size);
    }

    fn acquire(&self, thread_id: usize, size: usize) -> Address {
        let ret: Address = self.pr.lock().unwrap().get_new_pages(size);

        // XXX: Remove second predicate once non-JikesRVM GC is implemented
        if ret.is_zero() && cfg!(feature = "jikesrvm") {
            selected_plan::PLAN.control_collector_context.request();
            println!("Blocking for GC");
            block_for_gc(thread_id);
            println!("GC completed");
        }

        ret
    }

    fn in_space(&self, object: ObjectReference) -> bool {
        let page_resource = self.pr.lock().unwrap();
        let page_start = page_resource.get_start().as_usize();
        let page_extend = page_resource.get_extend();
        object.value() >= page_start && object.value() < page_start + page_extend
    }
}

impl ImmortalSpace {
    pub fn new() -> Self {
        ImmortalSpace {
            pr: Mutex::new(MonotonePageResource::new()),
        }
    }
}