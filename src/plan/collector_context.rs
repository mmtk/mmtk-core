use ::util::{Address, ObjectReference};
use ::plan::Phase;

pub trait CollectorContext {
    /// Notify that the collector context is registered and ready to execute.
    fn init(&mut self, id: usize);
    /// Allocate space for copying an object.
    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: usize) -> Address;
    /// Entry point for the collector context.
    fn run(&self);
    /// Perform a (local, i.e.per-collector) collection phase.
    fn collection_phase(&mut self, phase: Phase, primary: bool);
}