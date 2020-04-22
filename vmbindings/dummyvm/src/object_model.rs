use mmtk::vm::ObjectModel;
use mmtk::util::{Address, ObjectReference};
use mmtk::Allocator;
use mmtk::util::OpaquePointer;
use DummyVM;
use std::sync::atomic::AtomicU8;

pub struct VMObjectModel {}

impl ObjectModel<DummyVM> for VMObjectModel {
    const GC_BYTE_OFFSET: usize = 0;

    fn get_gc_byte(_object: ObjectReference) -> &'static AtomicU8 {
        unimplemented!()
    }

    fn copy(_from: ObjectReference, _allocator: Allocator, _tls: OpaquePointer) -> ObjectReference {
        unimplemented!()
    }

    fn copy_to(_from: ObjectReference, _to: ObjectReference, _region: Address) -> Address {
        unimplemented!()
    }

    fn get_reference_when_copied_to(_from: ObjectReference, _to: Address) -> ObjectReference {
        unimplemented!()
    }

    fn get_size_when_copied(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_align_when_copied(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_align_offset_when_copied(_object: ObjectReference) -> isize {
        unimplemented!()
    }

    fn get_current_size(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn get_next_object(_object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }

    unsafe fn get_object_from_start_address(_start: Address) -> ObjectReference {
        unimplemented!()
    }

    fn get_object_end_address(_object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn get_type_descriptor(_reference: ObjectReference) -> &'static [i8] {
        unimplemented!()
    }

    fn is_array(_object: ObjectReference) -> bool {
        unimplemented!()
    }

    fn is_primitive_array(_object: ObjectReference) -> bool {
        unimplemented!()
    }

    fn get_array_length(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn attempt_available_bits(_object: ObjectReference, _old: usize, _new: usize) -> bool {
        unimplemented!()
    }

    fn prepare_available_bits(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn write_available_byte(_object: ObjectReference, _val: u8) {
        unimplemented!()
    }

    fn read_available_byte(_object: ObjectReference) -> u8 {
        unimplemented!()
    }

    fn write_available_bits_word(_object: ObjectReference, val: usize) {
        unimplemented!()
    }

    fn read_available_bits_word(_object: ObjectReference) -> usize {
        unimplemented!()
    }

    fn gc_header_offset() -> isize {
        unimplemented!()
    }

    fn object_start_ref(_object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn ref_to_address(_object: ObjectReference) -> Address {
        unimplemented!()
    }

    fn is_acyclic(_typeref: ObjectReference) -> bool {
        unimplemented!()
    }

    fn dump_object(_object: ObjectReference) {
        unimplemented!()
    }

    fn get_array_base_offset() -> isize {
        unimplemented!()
    }

    fn array_base_offset_trapdoor<T>(_object: T) -> isize {
        unimplemented!()
    }

    fn get_array_length_offset() -> isize {
        unimplemented!()
    }
}
