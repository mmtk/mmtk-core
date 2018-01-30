use ::util::Address;
use ::util::ObjectReference;

pub trait Space {
    fn init(&self, heap_size: usize);

    fn acquire(&self, thread_id: usize, size: usize) -> Address;

    fn in_space(&self, object: ObjectReference) -> bool;
}

pub mod default {
    use ::plan::selected_plan;

    use ::util::Address;
    use ::util::ObjectReference;
    use ::util::heap::PageResource;

    use ::vm::Scheduling;
    use ::vm::VMScheduling;

    use std::sync::Mutex;

    pub fn init<T: PageResource>(pr: &Mutex<T>, heap_size: usize) {
        pr.lock().unwrap().init(heap_size);
    }

    pub fn acquire<T: PageResource>(pr: &Mutex<T>, thread_id: usize, size: usize) -> Address {
        let ret: Address = pr.lock().unwrap().get_new_pages(size);

        if ret.is_zero() {
            selected_plan::PLAN.control_collector_context.request();
            println!("Blocking for GC");
            VMScheduling::block_for_gc(thread_id);
            println!("GC completed");
        }

        ret
    }

    pub fn in_space<T: PageResource>(pr: &Mutex<T>, object: ObjectReference) -> bool {
        let page_resource = pr.lock().unwrap();
        let page_start = page_resource.get_start().as_usize();
        let page_extend = page_resource.get_extend();
        object.value() >= page_start && object.value() < page_start + page_extend
    }
}