use ::plan::Phase;
use ::util::{Address, ObjectReference};
use ::plan::Allocator;

use libc::c_void;

pub trait MutatorContext {
    fn collection_phase(&mut self, tls: *mut c_void, phase: &Phase, primary: bool);
    fn alloc(&mut self, size: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize, allocator: Allocator) -> Address;
    fn post_alloc(&mut self, refer: ObjectReference, type_refer: ObjectReference, bytes: usize,
                  allocator: Allocator);
    fn flush(&mut self) {
        self.flush_remembered_sets()
    }
    fn flush_remembered_sets(&mut self) {}
    fn get_tls(&self) -> *mut c_void;
    fn object_reference_write_slow(&mut self, _src: ObjectReference, _slot: Address, _value: ObjectReference) {
        unreachable!()
    }
    fn object_reference_try_compare_and_swap_slow(&mut self, _src: ObjectReference, _slot: Address, _old: ObjectReference, _new: ObjectReference) {
        unreachable!()
    }
    fn java_lang_reference_read_slow(&mut self, _ref: ObjectReference) -> ObjectReference {
        unreachable!()
    }
    fn deinit_mutator(&mut self) {
        self.flush();
    }
}