use libc::c_void;

pub trait Plan {
    fn new() -> Self;
    fn gc_init(&self, heap_size: usize);
    fn bind_mutator(&self, thread_id: usize) -> *mut c_void;
    fn do_collection(&self);
}

pub mod default {
    use std::thread;
    use libc::c_void;

    use ::policy::space::Space;
    use ::plan::mutator_context::MutatorContext;

    use super::super::selected_plan::PLAN;

    pub fn gc_init<T: Space>(space: &T, heap_size: usize) {
        space.init(heap_size);

        if !cfg!(feature = "jikesrvm") {
            thread::spawn(|| {
                PLAN.control_collector_context.run(0);
            });
        }
    }

    pub fn bind_mutator<'a, M: MutatorContext<'a, S>, S: Space>(thread_id: usize, space: &'a S) -> *mut c_void {
        Box::into_raw(Box::new(M::new(thread_id, space))) as *mut c_void
    }
}