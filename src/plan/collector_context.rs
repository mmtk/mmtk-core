use ::util::{Address, ObjectReference};

pub trait CollectorContext {
    /// Notify that the collector context is registered and ready to execute.
    fn init(&mut self, id: usize);
    /// Allocate space for copying an object.
    fn alloc_copy(original: ObjectReference, bytes: usize, align: usize, offset: usize, allocator: usize) -> Address;
    /// Entry point for the collector context.
    fn run();
}