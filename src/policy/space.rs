use ::util::address::Address;

pub trait Space {
    fn new() -> Self;

    fn init(&self, heap_size: usize);

    fn acquire(&self, thread_id: usize, size: usize) -> Address;
}