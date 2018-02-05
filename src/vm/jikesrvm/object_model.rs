use super::java_header_constants::{ADDRESS_BASED_HASHING, GC_HEADER_OFFSET, DYNAMIC_HASH_OFFSET,
    HASH_STATE_MASK, HASH_STATE_HASHED_AND_MOVED, ARRAY_BASE_OFFSET, ARRAY_LENGTH_OFFSET,
    HASHCODE_BYTES};
use super::java_header::*;
use super::memory_manager_constants::*;
use super::tib_layout_constants::*;
use super::entrypoint::*;
use super::unboxed_size_constants::*;
use super::JTOC_BASE;

use ::vm::object_model::ObjectModel;
use ::util::{Address, ObjectReference};
use ::util::constants::*;
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
        unsafe {
            let tib = Address::from_usize(object.to_address().load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            let mut size = if (rvm_type + IS_CLASS_TYPE_FIELD_OFFSET).load::<bool>() {
                (rvm_type + INSTANCE_SIZE_FIELD_OFFSET).load::<usize>()
            } else {
                let num_elements = Self::get_array_length(object);
                ARRAY_HEADER_SIZE
                    + (num_elements << (rvm_type + LOG_ELEMENT_SIZE_FIELD_OFFSET).load::<usize>())
            };

            if MOVES_OBJECTS {
                if ADDRESS_BASED_HASHING {
                    let hash_state = (object.value() as isize + STATUS_OFFSET) as usize
                        & HASH_STATE_MASK;
                    if hash_state == HASH_STATE_HASHED_AND_MOVED {
                        size += HASHCODE_BYTES;
                    }
                }
            }

            size
        }
    }

    fn get_next_object(object: ObjectReference) -> ObjectReference {
        unsafe {
            // XXX: It can't be this simple..
            Self::get_object_from_start_address(Self::get_object_end_address(object))
        }
    }

    unsafe fn get_object_from_start_address(start: Address) -> ObjectReference {
        let mut _start = start;

        /* Skip over any alignment fill */
        while _start.load::<usize>() == ALIGNMENT_VALUE {
            _start += size_of::<usize>();
        }

        (_start + OBJECT_REF_OFFSET).to_object_reference()
    }

    fn get_object_end_address(object: ObjectReference) -> Address {
        unsafe {
            let tib = Address::from_usize(object.to_address().load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            let mut size = if (rvm_type + IS_CLASS_TYPE_FIELD_OFFSET).load::<bool>() {
                (rvm_type + INSTANCE_SIZE_FIELD_OFFSET).load::<usize>()
            } else {
                let num_elements = Self::get_array_length(object);
                ARRAY_HEADER_SIZE
                    + (num_elements << (rvm_type + LOG_ELEMENT_SIZE_FIELD_OFFSET).load::<usize>())
            };

            if ADDRESS_BASED_HASHING && DYNAMIC_HASH_OFFSET {
                let hash_state = (object.value() as isize + STATUS_OFFSET) as usize
                    & HASH_STATE_MASK;
                if hash_state == HASH_STATE_HASHED_AND_MOVED {
                    size += HASHCODE_BYTES;
                }
            }
            object.to_address()
                + Address::from_usize(size).align_up(BYTES_IN_INT).as_usize()
                - OBJECT_REF_OFFSET
        }
    }

    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8] {
        unimplemented!()
    }

    fn is_array(object: ObjectReference) -> bool {
        unsafe {
            let tib = Address::from_usize(object.to_address().load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());
            (rvm_type + IS_ARRAY_TYPE_FIELD_OFFSET).load::<bool>()
        }
    }

    fn is_primitive_array(object: ObjectReference) -> bool {
        unsafe {
            // XXX: Is it OK to compare references like this?
            object.value() == (JTOC_BASE + LONG_ARRAY_FIELD_OFFSET).load::<usize>()
                || object.value() == (JTOC_BASE + INT_ARRAY_FIELD_OFFSET).load::<usize>()
                || object.value() == (JTOC_BASE + BYTE_ARRAY_FIELD_OFFSET).load::<usize>()
                || object.value() == (JTOC_BASE + INT_ARRAY_FIELD_OFFSET).load::<usize>()
                || object.value() == (JTOC_BASE + DOUBLE_ARRAY_FIELD_OFFSET).load::<usize>()
                || object.value() == (JTOC_BASE + FLOAT_ARRAY_FIELD_OFFSET).load::<usize>()
        }
    }

    fn get_array_length(object: ObjectReference) -> usize {
        let len_addr = object.to_address() + Self::get_array_length_offset();
        unsafe { len_addr.load::<usize>() }
    }

    fn attempt_available_bits(object: ObjectReference, old: usize, new: usize) -> bool {
        let loc = unsafe {
            &*((object.to_address() + STATUS_OFFSET).as_usize() as *mut AtomicUsize)
        };
        // XXX: Relaxed in OK on failure, right??
        loc.compare_exchange(old, new, Ordering::Release, Ordering::Relaxed).is_ok()
    }

    fn prepare_available_bits(object: ObjectReference) -> usize {
        let loc = unsafe {
            &*((object.to_address() + STATUS_OFFSET).as_usize() as *mut AtomicUsize)
        };
        loc.load(Ordering::Acquire)
    }

    // XXX: Supposedly none of the 4 methods below need to use atomic loads/stores
    fn write_available_byte(object: ObjectReference, val: u8) {
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).store::<u8>(val);
        }
    }

    fn read_available_byte(object: ObjectReference) -> u8 {
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).load::<u8>()
        }
    }

    fn write_available_bits_word(object: ObjectReference, val: usize) {
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).store::<usize>(val);
        }
    }

    fn read_available_bits_word(object: ObjectReference) -> usize {
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).load::<usize>()
        }
    }

    fn GC_HEADER_OFFSET() -> isize {
        GC_HEADER_OFFSET
    }

    fn object_start_ref(object: ObjectReference) -> Address {
        if MOVES_OBJECTS {
            if ADDRESS_BASED_HASHING && !DYNAMIC_HASH_OFFSET {
                let hash_state = unsafe {
                    (object.to_address() + STATUS_OFFSET).load::<usize>() & HASH_STATE_MASK
                };
                if hash_state == HASH_STATE_HASHED_AND_MOVED {
                    return object.to_address() - (OBJECT_REF_OFFSET + HASHCODE_BYTES);
                }
            }
        }
        object.to_address() - OBJECT_REF_OFFSET
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
        panic!("This should (?) never be called")
    }

    fn get_array_length_offset() -> isize {
        ARRAY_LENGTH_OFFSET
    }
}
