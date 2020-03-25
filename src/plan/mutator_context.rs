use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::plan::Allocator;
use ::util::OpaquePointer;

pub trait MutatorContext {
    fn collection_phase(&mut self, tls: OpaquePointer, phase: &Phase, primary: bool);
    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize,
                  allocator: Allocator);
    fn flush_remembered_sets(&mut self) {}
    fn get_tls(&self) -> OpaquePointer;
}