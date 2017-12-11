use ::util::address::Address;

pub trait PageResource {
    fn new() -> Self;

    fn init(&mut self, heap_size: usize);

    fn get_new_pages(&mut self, size: usize) -> Address;
}