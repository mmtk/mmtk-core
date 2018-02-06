use ::util::{Address, ObjectReference};
use ::plan::{Phase, Allocator};
use ::plan::selected_plan::SelectedConstraints::*;

pub trait CollectorContext {
    fn new() -> Self;
    /// Notify that the collector context is registered and ready to execute.
    fn init(&mut self, id: usize);
    /// Allocate space for copying an object.
    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    /// Entry point for the collector context.
    fn run(&mut self, thread_id: usize);
    /// Perform a (local, i.e. per-collector) collection phase.
    fn collection_phase(&mut self, thread_id: usize, phase: &Phase, primary: bool);
    /// Unique identifier for this collector context.
    fn get_id(&self) -> usize;

    fn copy_check_allocator(&self, from: ObjectReference, bytes: usize, align: usize,
                            allocator: Allocator) -> Allocator {
        let large = ::util::alloc::allocator::get_maximum_aligned_size(bytes, align,
            ::util::alloc::allocator::MIN_ALIGNMENT) > MAX_NON_LOS_COPY_BYTES;
        if large { Allocator::Los } else { allocator }
    }

    fn post_copy(&self, obj: ObjectReference, tib: Address, bytes: usize, allocator: Allocator) {
        unimplemented!()
    }
}