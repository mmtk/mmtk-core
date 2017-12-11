use ::util::address::Address;

pub trait Space {
    fn new() -> Self;

    fn init(&self, heap_size: usize);

    fn acquire(&self, size: usize) -> Address;
}