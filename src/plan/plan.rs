use libc::c_void;
use ::util::ObjectReference;
use ::plan::{MutatorContext, CollectorContext, TraceLocal};

pub trait Plan {
    type MutatorT: MutatorContext;
    type TraceLocalT: TraceLocal;
    type CollectorT: CollectorContext;

    fn new() -> Self;
    fn gc_init(&self, heap_size: usize);
    fn bind_mutator(&self, thread_id: usize) -> *mut c_void;
    fn will_never_move(&self, object: ObjectReference) -> bool;
}

#[repr(i32)]
pub enum Allocator {
    Default = 0,
    NonReference = 1,
    NonMoving = 2,
    Immortal = 3,
    Los = 4,
    PrimitiveLos = 5,
    GcSpy = 6,
    Code = 7,
    LargeCode = 8,
    Allocators = 9,
    DefaultSite = -1,
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

    pub fn bind_mutator<T: MutatorContext>(ctx: T) -> *mut c_void {
        Box::into_raw(Box::new(ctx)) as *mut c_void
    }
}