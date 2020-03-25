use ::util::{Address, ObjectReference};
use ::plan::{Phase, Allocator};
use ::plan::selected_plan::SelectedConstraints::*;
use ::util::OpaquePointer;
use mmtk::MMTK;
use vm::VMBinding;

pub trait CollectorContext<VM: VMBinding> {
    fn new(mmtk: &'static MMTK<VM>) -> Self;
    /// Notify that the collector context is registered and ready to execute.
    fn init(&mut self, tls: OpaquePointer);
    /// Allocate space for copying an object.
    fn alloc_copy(&mut self, original: ObjectReference, bytes: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    /// Entry point for the collector context.
    fn run(&mut self, tls: OpaquePointer);
    /// Perform a (local, i.e. per-collector) collection phase.
    fn collection_phase(&mut self, tls: OpaquePointer, phase: &Phase, primary: bool);
    /// Unique identifier for this collector context.
    fn get_tls(&self) -> OpaquePointer;

    fn copy_check_allocator(&self, _from: ObjectReference, bytes: usize, align: usize,
                            allocator: Allocator) -> Allocator {
        let large = ::util::alloc::allocator::get_maximum_aligned_size(bytes, align,
            ::util::alloc::allocator::MIN_ALIGNMENT) > MAX_NON_LOS_COPY_BYTES;
        if large { Allocator::Los } else { allocator }
    }

    // TODO: the parameter tib seems quite JikesRVM specific?
    fn post_copy(&self, _obj: ObjectReference, _tib: Address, _bytes: usize, _allocator: Allocator) {
        unreachable!()
    }
}