use super::java_header_constants::*;
use super::java_header::*;

use ::vm::object_model::ObjectModel;
use ::util::{Address, ObjectReference};
use ::plan::Allocator;
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct VMObjectModel {}

impl ObjectModel for VMObjectModel {
    fn copy(from: ObjectReference, allocator: Allocator) -> ObjectReference {
        unimplemented!()
    }

    fn copy_to(from: ObjectReference, to: ObjectReference, region: Address) -> Address {
        unimplemented!()
    }

    fn get_reference_when_copied_to(from: ObjectReference, to: Address) -> ObjectReference {
        unimplemented!()
    }

    fn get_size_when_copied(object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_align_when_copied(object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_align_offset_when_copied(object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_current_size(object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_next_object(object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }

    fn get_object_from_start_address(start: Address) -> ObjectReference {
        unimplemented!()
    }

    fn get_object_end_address(object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8] {
        unimplemented!()
    }

    fn is_array(object: ObjectReference) -> bool {
        unimplemented!()
    }

    fn is_primitive_array(object: ObjectReference) -> bool {
        unimplemented!()
    }

    fn get_array_length(object: ObjectReference) -> usize {
        let len_addr = object.to_address() + Self::get_array_length_offset();
        unsafe { len_addr.load::<usize>() }
    }

    fn attempt_available_bits(object: ObjectReference, old: usize, new: usize) -> bool {
        let loc = unsafe {
            &*((object.to_address() + STATUS_OFFSET).as_usize() as *mut AtomicUsize)
        };
        // XXX: What are the ordering requirements, anyway?
        loc.compare_and_swap(old, new, Ordering::SeqCst) == old
    }

    fn prepare_available_bits(object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn write_available_byte(object: ObjectReference, val: u8) {
        unimplemented!()
    }

    fn read_available_byte(object: ObjectReference) -> u8 {
        unimplemented!()
    }

    fn write_available_bits_word(object: ObjectReference, val: usize) {
        unimplemented!()
    }

    fn read_available_bits_word(object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn GC_HEADER_OFFSET() -> isize {
        GC_HEADER_OFFSET
    }

    fn object_start_ref(object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn ref_to_address(object: ObjectReference) -> Address {
        object.to_address() + TIB_OFFSET
    }

    fn is_acyclic(typeref: ObjectReference) -> bool {
        unimplemented!()
    }

    fn dump_object(object: ObjectReference) {
        unimplemented!()
    }

    fn get_array_base_offset() -> isize {
        ARRAY_BASE_OFFSET
    }

    fn array_base_offset_trapdoor<T>(o: T) -> isize {
        unimplemented!()
    }

    fn get_array_length_offset() -> isize {
        ARRAY_LENGTH_OFFSET
    }
}
