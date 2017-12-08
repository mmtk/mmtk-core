use libc::c_void;

pub trait Plan {
    fn gc_init(heap_size: usize);
    fn bind_mutator(thread_id: usize) -> *mut c_void;
}