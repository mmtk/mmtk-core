use ::plan::Phase;
use ::util::Address;
use ::policy::space::Space;

pub trait MutatorContext<'a, T:Space> {
    fn new(thread_id: usize, space: &'a T) -> Self;
    fn collection_phase(&mut self, phase: Phase, primary:bool);
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address;
}