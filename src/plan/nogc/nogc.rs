use std::sync::Mutex;
use ::util::heap::MonotonePageResource;
use ::util::heap::PageResource;

use ::util::alloc::bumpallocator::BumpAllocator;
use ::util::alloc::allocator::Allocator;

use libc::c_void;

use ::policy::space::Space;
use ::policy::immortalspace::ImmortalSpace;

use ::plan::Plan;

lazy_static! {
    pub static ref SPACE: ImmortalSpace = ImmortalSpace::new();
}
pub type NoGCMutator<'a> = BumpAllocator<'a,ImmortalSpace>;

pub struct NoGC{}

impl Plan for NoGC {
    fn gc_init(heap_size: usize) {
        SPACE.init(heap_size);
    }

    fn bind_mutator(thread_id: usize) -> *mut c_void {
        Box::into_raw(Box::new(NoGCMutator::new(thread_id, &SPACE))) as *mut c_void
    }
}