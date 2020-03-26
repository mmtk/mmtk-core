use libc::c_void;
use mmtk::vm::ObjectModel;
use mmtk::util::{Address, ObjectReference};
use mmtk::Allocator;
use mmtk::util::OpaquePointer;
use DummyVM;
use std::sync::atomic::AtomicU8;

pub struct VMObjectModel {}

impl ObjectModel<DummyVM> for VMObjectModel {
    const GC_BYTE_OFFSET: usize = 0;

    fn get_gc_byte(o: ObjectReference) -> &'static AtomicU8 {
        unimplemented!()
    }

    fn copy(from: ObjectReference, allocator: Allocator, tls: OpaquePointer) -> ObjectReference {
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

    fn get_align_offset_when_copied(object: ObjectReference) -> isize {
        unimplemented!()
    }

    fn get_current_size(object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_next_object(object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }

    unsafe fn get_object_from_start_address(start: Address) -> ObjectReference {
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
        unimplemented!()
    }

    fn attempt_available_bits(object: ObjectReference, old: usize, new: usize) -> bool {
        unimplemented!()
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

    fn gc_header_offset() -> isize {
        unimplemented!()
    }

    fn object_start_ref(object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn ref_to_address(object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn is_acyclic(typeref: ObjectReference) -> bool {
        unimplemented!()
    }

    fn dump_object(object: ObjectReference) {
        unimplemented!()
    }

    fn get_array_base_offset() -> isize {
        unimplemented!()
    }

    fn array_base_offset_trapdoor<T>(o: T) -> isize {
        unimplemented!()
    }

    fn get_array_length_offset() -> isize {
        unimplemented!()
    }
}
