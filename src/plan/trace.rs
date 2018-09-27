use ::util::{Address, ObjectReference};
use ::util::shared_queue::SharedQueue;
use crossbeam_deque::Stealer;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

pub struct Trace {
    pub values: SharedQueue<ObjectReference>,
    pub root_locations: SharedQueue<Address>,
}

impl Trace {
    pub fn new() -> Self {
        Trace {
            values: SharedQueue::new(),
            root_locations: SharedQueue::new(),
        }
    }

    pub fn prepare(&mut self) {}
}