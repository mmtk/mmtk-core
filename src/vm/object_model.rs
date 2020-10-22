use crate::Allocator;
use crate::plan::CopyContext;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use std::sync::atomic::AtomicU8;

/// https://github.com/JikesRVM/JikesRVM/blob/master/MMTk/src/org/mmtk/vm/ObjectModel.java
pub trait ObjectModel<VM: VMBinding> {
    const GC_BYTE_OFFSET: usize;
    fn get_gc_byte(o: ObjectReference) -> &'static AtomicU8;
    fn copy(from: ObjectReference, allocator: Allocator, copy_context: &mut impl CopyContext) -> ObjectReference;
    fn copy_to(from: ObjectReference, to: ObjectReference, region: Address) -> Address;
    fn get_reference_when_copied_to(from: ObjectReference, to: Address) -> ObjectReference;
    fn get_size_when_copied(object: ObjectReference) -> usize;
    fn get_align_when_copied(object: ObjectReference) -> usize;
    fn get_align_offset_when_copied(object: ObjectReference) -> isize;
    fn get_current_size(object: ObjectReference) -> usize;
    fn get_next_object(object: ObjectReference) -> ObjectReference;
    /// # Safety
    /// We would expect ObjectReferences point to valid objects,
    /// but an arbitrary Address may not reside an object. This conversion is unsafe,
    /// and it is the user's responsibility to ensure the safety.
    unsafe fn get_object_from_start_address(start: Address) -> ObjectReference;
    fn get_object_end_address(object: ObjectReference) -> Address;
    // FIXME: determine lifetime, returns byte[]
    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8];
    fn is_array(object: ObjectReference) -> bool;
    fn is_primitive_array(object: ObjectReference) -> bool;
    fn get_array_length(object: ObjectReference) -> usize;

    // TODO: The methods below should be replaced with a getter of `&AtomicUsize` or `&AtomicU8`,
    // following the pattern of get_gc_byte()
    fn attempt_available_bits(object: ObjectReference, old: usize, new: usize) -> bool;
    fn prepare_available_bits(object: ObjectReference) -> usize;
    fn write_available_byte(object: ObjectReference, val: u8);
    fn read_available_byte(object: ObjectReference) -> u8;
    fn write_available_bits_word(object: ObjectReference, val: usize);
    fn read_available_bits_word(object: ObjectReference) -> usize;

    // Offset
    fn gc_header_offset() -> isize;
    fn object_start_ref(object: ObjectReference) -> Address;
    fn ref_to_address(object: ObjectReference) -> Address;
    fn is_acyclic(typeref: ObjectReference) -> bool;
    fn dump_object(object: ObjectReference);
    fn get_array_base_offset() -> isize;
    fn array_base_offset_trapdoor<T>(o: T) -> isize;
    fn get_array_length_offset() -> isize;
}
