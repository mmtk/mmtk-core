use ::policy::space::Space;
use ::util::address::Address;

pub struct CopySpace {}

impl Space for CopySpace {
    fn new() -> Self {
        CopySpace {}
    }

    fn init(&self, heap_size: usize) {}

    fn acquire(&self, thread_id: usize, size: usize) -> Address {
        unsafe { Address::zero() }
    }
}