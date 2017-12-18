use ::util::Address;
use ::util::ObjectReference;

pub trait Space {
    fn new() -> Self;

    fn init(&self, heap_size: usize);

    fn acquire(&self, thread_id: usize, size: usize) -> Address;

    fn in_space(&self, object: ObjectReference) -> bool;
}