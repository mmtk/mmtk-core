use ::plan::Phase;
use ::util::Address;

pub trait MutatorContext {
    fn collection_phase(&mut self, phase: Phase, primary: bool);
    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address;
}