use ::policy::space::Space;
use ::util::{Address, ObjectReference};

pub struct CopySpace {
    from_space: bool
}

impl Space for CopySpace {
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

impl CopySpace {
    pub fn new(from_space: bool) -> Self {
        CopySpace {
            from_space
        }
    }

    pub fn prepare(&mut self, from_space: bool) {
        self.from_space = from_space;
    }
}