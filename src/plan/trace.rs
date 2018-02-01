use ::util::{Address, ObjectReference};
use ::util::global_pool;
use std::thread::JoinHandle;
use std::sync::mpsc::Sender;
use crossbeam_deque::Stealer;

pub struct Trace {
    pub values: (Stealer<ObjectReference>, Sender<ObjectReference>),
    pub root_locations: (Stealer<Address>, Sender<Address>),
}

impl Trace {
    pub fn new() -> Self {
        Trace {
            values: global_pool::new(),
            root_locations: global_pool::new(),
        }
    }

    pub fn prepare(&mut self) {
    }

    pub fn get_value_pool(&self) -> (Stealer<ObjectReference>, Sender<ObjectReference>) {
        self.values.clone()
    }

    pub fn get_root_location_pool(&self) -> (Stealer<Address>, Sender<Address>) {
        self.root_locations.clone()
    }
}