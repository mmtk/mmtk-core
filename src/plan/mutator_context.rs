use ::plan::Phase;
use ::util::Address;
use ::plan::Allocator;

pub trait MutatorContext {
    fn collection_phase(&mut self, thread_id: usize, phase: &Phase, primary: bool);
    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    fn flush_remembered_sets() {
    }
}