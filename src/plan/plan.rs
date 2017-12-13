use libc::c_void;

pub trait Plan {
    fn new() -> Self;
    fn gc_init(&self, heap_size: usize);
    fn bind_mutator(&self, thread_id: usize) -> *mut c_void;
}