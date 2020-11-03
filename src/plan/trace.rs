use crate::util::queue::SharedQueue;
use crate::util::{Address, ObjectReference};

/// This trait describes the core functionality for a transitive
/// closure over the heap. Any class implementing this trait holds the global state, TraceLocal
/// and its implementers handle per-thread state.
pub struct Trace {
    pub values: SharedQueue<ObjectReference>,
    pub root_locations: SharedQueue<Address>,
}

impl Trace {
    // It is possible that a plan does not use Trace (such as NoGC)
    #[allow(unused)]
    pub fn new() -> Self {
        Trace {
            values: SharedQueue::new(),
            root_locations: SharedQueue::new(),
        }
    }

    // FIXME: temporarily disable the warning. I will do a separte PR for this.
    #[allow(unused)]
    pub fn prepare(&mut self) {
        // TODO: we should reset shared queue here, and we should call prepare() in prepare phase
    }
}

impl Default for Trace {
    fn default() -> Self {
        Self::new()
    }
}
