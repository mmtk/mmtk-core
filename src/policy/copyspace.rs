use ::policy::space::Space;
use ::util::{Address, ObjectReference};

pub struct CopySpace {}

impl Space for CopySpace {
    fn new() -> Self {
        CopySpace {}
    }

    fn init(&self, heap_size: usize) {
        unimplemented!()
    }

    fn acquire(&self, thread_id: usize, size: usize) -> Address {
        unimplemented!()
    }
    fn in_space(&self, object: ObjectReference) -> bool {
        unimplemented!()
    }
}