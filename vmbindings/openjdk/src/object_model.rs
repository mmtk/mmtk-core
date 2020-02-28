use libc::c_void;
use mmtk::vm::*;
use mmtk::Allocator;
use mmtk::util::{Address, ObjectReference};
use mmtk::util::OpaquePointer;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use super::UPCALLS;
use OpenJDK;
use mmtk::CollectorContext;

pub struct VMObjectModel {}

impl ObjectModel<OpenJDK> for VMObjectModel {
    #[cfg(target_pointer_width = "64")]
    const GC_BYTE_OFFSET: usize = 56;
    #[cfg(target_pointer_width = "32")]
    const GC_BYTE_OFFSET: usize = 0;
    fn get_gc_byte(o: ObjectReference) -> &'static AtomicU8 {
        unsafe {
            &*(o.to_address() + Self::GC_BYTE_OFFSET / 8).to_ptr::<AtomicU8>()
        }
    }
    fn copy(from: ObjectReference, allocator: Allocator, tls: OpaquePointer) -> ObjectReference {
        let bytes = unsafe { ((*UPCALLS).get_object_size)(from) };
        let context = unsafe { <OpenJDK as VMBinding>::VMActivePlan::collector(tls) };
        let dst = context.alloc_copy(from, bytes, ::std::mem::size_of::<usize>(), 0, allocator);
        // Copy
        let src = from.to_address();
        for i in 0..bytes {
            unsafe { (dst + i).store((src + i).load::<u8>()) };
        }
        let to_obj = unsafe { dst.to_object_reference() };
        context.post_copy(to_obj, unsafe { Address::zero() }, bytes, allocator);
        to_obj
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
        unsafe {
            object.to_address().compare_exchange::<AtomicUsize>(old, new, Ordering::SeqCst, Ordering::SeqCst).is_ok()
        }
    }

    fn prepare_available_bits(object: ObjectReference) -> usize {
        unsafe { object.to_address().load() }
    }

    fn write_available_byte(object: ObjectReference, val: u8) {
        unimplemented!()
    }

    fn read_available_byte(object: ObjectReference) -> u8 {
        unimplemented!()
    }

    fn write_available_bits_word(object: ObjectReference, val: usize) {
        unsafe { object.to_address().atomic_store::<AtomicUsize>(val, Ordering::SeqCst) }
    }

    fn read_available_bits_word(object: ObjectReference) -> usize {
        unsafe { object.to_address().atomic_load::<AtomicUsize>(Ordering::SeqCst) }
    }

    fn GC_HEADER_OFFSET() -> isize {
        unimplemented!()
    }

    fn object_start_ref(object: ObjectReference) -> Address {
        object.to_address()
    }

    fn ref_to_address(object: ObjectReference) -> Address {
        object.to_address()
    }

    fn is_acyclic(typeref: ObjectReference) -> bool {
        unimplemented!()
    }

    fn dump_object(object: ObjectReference) {
        unsafe {
            ((*UPCALLS).dump_object)(object);
        }
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
