use ::util::{Address, ObjectReference};
use ::plan::{Phase, Allocator};
use ::plan::selected_plan::SelectedConstraints::*;

use libc::c_void;

pub trait CollectorContext {
    fn new() -> Self;
    /// Notify that the collector context is registered and ready to execute.
    fn init(&mut self, tls: *mut c_void);
    /// Allocate space for copying an object.
    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    /// Entry point for the collector context.
    fn run(&mut self, tls: *mut c_void);
    /// Perform a (local, i.e. per-collector) collection phase.
    fn collection_phase(&mut self, tls: *mut c_void, phase: &Phase, primary: bool);
    /// Unique identifier for this collector context.
    fn get_tls(&self) -> *mut c_void;

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