extern crate libc;
use libc::*;

use super::java_header_constants::{ADDRESS_BASED_HASHING, GC_HEADER_OFFSET, DYNAMIC_HASH_OFFSET,
    HASH_STATE_MASK, HASH_STATE_HASHED_AND_MOVED, ARRAY_BASE_OFFSET, ARRAY_LENGTH_OFFSET,
    HASHCODE_BYTES, HASH_STATE_UNHASHED, HASH_STATE_HASHED, HASHCODE_OFFSET, ALIGNMENT_MASK};
use super::java_header::*;
use super::memory_manager_constants::*;
use super::tib_layout_constants::*;
use super::entrypoint::*;
use super::super::unboxed_size_constants::*;
use super::java_size_constants::{BYTES_IN_INT, BYTES_IN_DOUBLE};
use super::class_loader_constants::*;
use super::JTOC_BASE;

use super::super::ActivePlan;
use super::active_plan::VMActivePlan;

use ::vm::object_model::ObjectModel;
use ::util::{Address, ObjectReference};
use ::util::alloc::allocator::fill_alignment_gap;
use ::util::constants::{};
use ::plan::{Allocator, CollectorContext};
use std::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};

/** Should we gather stats on hash code state transitions for address-based hashing? */
const HASH_STATS: bool = false;
/** count number of Object.hashCode() operations */
static HASH_REQUESTS: AtomicUsize = AtomicUsize::new(0);
/** count transitions from UNHASHED to HASHED */
static HASH_TRANSITION1: AtomicUsize = AtomicUsize::new(0);
/** count transitions from HASHED to HASHED_AND_MOVED */
static HASH_TRANSITION2: AtomicUsize = AtomicUsize::new(0);

/** Whether to pack bytes and shorts into 32bit fields*/
const PACKED: bool = true;

pub struct VMObjectModel {}

impl ObjectModel for VMObjectModel {
    #[inline(always)]
    fn copy(from: ObjectReference, allocator: Allocator, thread_id: usize) -> ObjectReference {
        trace!("ObjectModel.copy");
        unsafe {
            trace!("getting tib");
            let tib = Address::from_usize((from.to_address() + TIB_OFFSET).load::<usize>());
            trace!("getting type, tib={:?}", tib);
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            trace!("Is it a class?");
            if (rvm_type + IS_CLASS_TYPE_FIELD_OFFSET).load::<bool>() {
                trace!("... yes");
                Self::copy_scalar(from, tib, rvm_type, allocator, thread_id)
            } else {
                trace!("... no");
                Self::copy_array(from, tib, rvm_type, allocator, thread_id)
            }
        }
    }

    #[inline(always)]
    fn copy_to(from: ObjectReference, to: ObjectReference, region: Address) -> Address {
        trace!("ObjectModel.copy_to");
        unsafe {
            let tib = Address::from_usize((from.to_address() + TIB_OFFSET).load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());
            let mut bytes: usize = 0;

            let copy = from != to;

            if copy {
                let size = Self::bytes_required_when_copied(from, rvm_type);
                Self::move_object(Address::zero(), from, to, bytes, rvm_type);
            } else {
                bytes = Self::bytes_used(from, rvm_type);
            }

            let start = Self::object_start_ref(to);
            fill_alignment_gap(region, start);

            start + bytes
        }
    }

    fn get_reference_when_copied_to(from: ObjectReference, to: Address) -> ObjectReference {
        trace!("ObjectModel.get_reference_when_copied");
        let mut res = to;
        if ADDRESS_BASED_HASHING && !DYNAMIC_HASH_OFFSET {
            unsafe {
                let hash_state = (from.to_address() + STATUS_OFFSET).load::<usize>()
                    & HASH_STATE_MASK;
                if hash_state != HASH_STATE_UNHASHED {
                    res += HASHCODE_BYTES;
                }
            }
        }

        unsafe { (to + OBJECT_REF_OFFSET).to_object_reference() }
    }

    fn get_size_when_copied(object: ObjectReference) -> usize {
        trace!("ObjectModel.get_size_when_copied");
        unsafe {
            let tib = Address::from_usize((object.to_address() + TIB_OFFSET).load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            Self::bytes_required_when_copied(object, rvm_type)
        }
    }

    fn get_align_when_copied(object: ObjectReference) -> usize {
        trace!("ObjectModel.get_align_when_copied");
        unsafe {
            let tib = Address::from_usize((object.to_address() + TIB_OFFSET).load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            if (rvm_type + IS_ARRAY_TYPE_FIELD_OFFSET).load::<bool>() {
                Self::get_alignment_array(rvm_type)
            } else {
                Self::get_alignment_class(rvm_type)
            }
        }
    }

    fn get_align_offset_when_copied(object: ObjectReference) -> isize {
        trace!("ObjectModel.get_align_offset_when_copied");
        unsafe {
            let tib = Address::from_usize((object.to_address() + TIB_OFFSET).load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            if (rvm_type + IS_ARRAY_TYPE_FIELD_OFFSET).load::<bool>() {
                Self::get_offset_for_alignment_array(object, rvm_type)
            } else {
                Self::get_offset_for_alignment_class(object, rvm_type)
            }
        }
    }

    fn get_current_size(object: ObjectReference) -> usize {
        trace!("ObjectModel.get_current_size");
        unsafe {
            let tib = Address::from_usize((object.to_address() + TIB_OFFSET).load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            Self::bytes_used(object, rvm_type)
        }
    }

    fn get_next_object(object: ObjectReference) -> ObjectReference {
        trace!("ObjectModel.get_next_object");
        unsafe {
            // XXX: It can't be this simple..
            Self::get_object_from_start_address(Self::get_object_end_address(object))
        }
    }

    unsafe fn get_object_from_start_address(start: Address) -> ObjectReference {
        trace!("ObjectModel.get_object_from_start_address");
        let mut _start = start;

        /* Skip over any alignment fill */
        while _start.load::<usize>() == ALIGNMENT_VALUE {
            _start += size_of::<usize>();
        }

        (_start + OBJECT_REF_OFFSET).to_object_reference()
    }

    fn get_object_end_address(object: ObjectReference) -> Address {
        trace!("ObjectModel.get_object_end_address");
        unsafe {
            let tib = Address::from_usize((object.to_address() + TIB_OFFSET).load::<usize>());
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
                let hash_state = (object.to_address() + STATUS_OFFSET).load::<usize>()
                    & HASH_STATE_MASK;
                if hash_state == HASH_STATE_HASHED_AND_MOVED {
                    size += HASHCODE_BYTES;
                }
            }
            object.to_address()
                + Address::from_usize(size).align_up(BYTES_IN_INT).as_usize()
                + (-OBJECT_REF_OFFSET)
        }
    }

    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8] {
        unimplemented!()
    }

    fn is_array(object: ObjectReference) -> bool {
        trace!("ObjectModel.is_array");
        unsafe {
            let tib = Address::from_usize((object.to_address() + TIB_OFFSET).load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());
            (rvm_type + IS_ARRAY_TYPE_FIELD_OFFSET).load::<bool>()
        }
    }

    fn is_primitive_array(object: ObjectReference) -> bool {
        trace!("ObjectModel.is_primitive_array");
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

    #[inline(always)]
    fn get_array_length(object: ObjectReference) -> usize {
        trace!("ObjectModel.get_array_length");
        let len_addr = object.to_address() + Self::get_array_length_offset();
        unsafe { len_addr.load::<usize>() }
    }

    fn attempt_available_bits(object: ObjectReference, old: usize, new: usize) -> bool {
        trace!("ObjectModel.attempt_available_bits");
        let loc = unsafe {
            &*((object.to_address() + STATUS_OFFSET).as_usize() as *mut AtomicUsize)
        };
        // XXX: Relaxed in OK on failure, right??
        loc.compare_exchange(old, new, Ordering::Release, Ordering::Relaxed).is_ok()
    }

    fn prepare_available_bits(object: ObjectReference) -> usize {
        trace!("ObjectModel.prepare_available_bits");
        let loc = unsafe {
            &*((object.to_address() + STATUS_OFFSET).as_usize() as *mut AtomicUsize)
        };
        loc.load(Ordering::Acquire)
    }

    // XXX: Supposedly none of the 4 methods below need to use atomic loads/stores
    fn write_available_byte(object: ObjectReference, val: u8) {
        trace!("ObjectModel.write_available_byte");
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).store::<u8>(val);
        }
    }

    fn read_available_byte(object: ObjectReference) -> u8 {
        trace!("ObjectModel.read_available_byte");
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).load::<u8>()
        }
    }

    fn write_available_bits_word(object: ObjectReference, val: usize) {
        trace!("ObjectModel.write_available_bits_word");
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).store::<usize>(val);
        }
    }

    fn read_available_bits_word(object: ObjectReference) -> usize {
        trace!("ObjectModel.read_available_bits_word");
        unsafe {
            (object.to_address() + AVAILABLE_BITS_OFFSET).load::<usize>()
        }
    }

    fn GC_HEADER_OFFSET() -> isize {
        GC_HEADER_OFFSET
    }

    #[inline(always)]
    fn object_start_ref(object: ObjectReference) -> Address {
        trace!("ObjectModel.object_start_ref");
        if MOVES_OBJECTS {
            if ADDRESS_BASED_HASHING && !DYNAMIC_HASH_OFFSET {
                let hash_state = unsafe {
                    (object.to_address() + STATUS_OFFSET).load::<usize>() & HASH_STATE_MASK
                };
                if hash_state == HASH_STATE_HASHED_AND_MOVED {
                    return object.to_address() + (-(OBJECT_REF_OFFSET + HASHCODE_BYTES as isize));
                }
            }
        }
        object.to_address() + (-OBJECT_REF_OFFSET)
    }

    fn ref_to_address(object: ObjectReference) -> Address {
        object.to_address() + TIB_OFFSET
    }

    #[inline(always)]
    fn is_acyclic(typeref: ObjectReference) -> bool {
        trace!("ObjectModel.is_acyclic");
        unsafe {
            let tib = Address::from_usize((typeref.to_address() + TIB_OFFSET).load::<usize>());
            let rvm_type = Address::from_usize((tib + TIB_TYPE_INDEX * BYTES_IN_ADDRESS)
                .load::<usize>());

            let is_array = (rvm_type + IS_ARRAY_TYPE_FIELD_OFFSET).load::<bool>();
            let is_class = (rvm_type + IS_CLASS_TYPE_FIELD_OFFSET).load::<bool>();
            if !is_array && !is_class {
                true
            } else if is_array {
                (rvm_type + RVM_ARRAY_ACYCLIC_OFFSET).load::<bool>()
            } else {
                let modifiers = (rvm_type + RVM_CLASS_MODIFIERS_OFFSET).load::<u16>();
                (modifiers & ACC_FINAL != 0)
                    && (rvm_type + RVM_CLASS_ACYCLIC_OFFSET).load::<bool>()
            }
        }
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

impl VMObjectModel {
    #[inline(always)]
    fn copy_scalar(from: ObjectReference, tib: Address, rvm_type: Address,
                   immut_allocator: Allocator, thread_id: usize) -> ObjectReference {
        trace!("VMObjectModel.copy_scalar");
        let bytes = Self::bytes_required_when_copied_class(from, rvm_type);
        let align = Self::get_alignment_class(rvm_type);
        let offset = Self::get_offset_for_alignment_class(from, rvm_type);
        let context = unsafe { VMActivePlan::collector(thread_id) };
        let allocator = context.copy_check_allocator(from, bytes, align, immut_allocator);
        let region = context.alloc_copy(from, bytes, align, offset, allocator);

        let to_obj = Self::move_object(region, from, unsafe {Address::zero().to_object_reference()},
                                       bytes, rvm_type);
        context.post_copy(to_obj, tib, bytes, allocator);
        to_obj
    }

    #[inline(always)]
    fn copy_array(from: ObjectReference, tib: Address, rvm_type: Address,
                  immut_allocator: Allocator, thread_id: usize) -> ObjectReference {
        trace!("VMObjectModel.copy_array");
        let bytes = Self::bytes_required_when_copied_array(from, rvm_type);
        let align = Self::get_alignment_array(rvm_type);
        let offset = Self::get_offset_for_alignment_array(from, rvm_type);
        let context = unsafe { VMActivePlan::collector(thread_id) };
        let allocator = context.copy_check_allocator(from, bytes, align, immut_allocator);
        let region = context.alloc_copy(from, bytes, align, offset, allocator);

        let to_obj = Self::move_object(region, from, unsafe {Address::zero().to_object_reference()},
                                       bytes, rvm_type);
        context.post_copy(to_obj, tib, bytes, allocator);
        // XXX: Do not sync icache/dcache because we do not support PowerPC
        to_obj
    }

    #[inline(always)]
    fn bytes_required_when_copied(object: ObjectReference, rvm_type: Address) -> usize {
        trace!("VMObjectModel.bytes_required_when_copied");
        unsafe {
            if (rvm_type + IS_CLASS_TYPE_FIELD_OFFSET).load::<bool>() {
                Self::bytes_required_when_copied_class(object, rvm_type)
            } else {
                Self::bytes_required_when_copied_array(object, rvm_type)
            }
        }
    }

    #[inline(always)]
    fn bytes_required_when_copied_class(object: ObjectReference, rvm_type: Address) -> usize {
        let mut size = unsafe { (rvm_type + INSTANCE_SIZE_FIELD_OFFSET).load::<usize>() };
        trace!("bytes_required_when_copied_class: instance size={}", size);

        if ADDRESS_BASED_HASHING {
            let hash_state = unsafe {
                (object.to_address() + STATUS_OFFSET).load::<usize>()
                    & HASH_STATE_MASK
                };
            if hash_state != HASH_STATE_UNHASHED {
                size += HASHCODE_BYTES;
            }
        }

        trace!("bytes_required_when_copied_class: returned size={}", size);
        size
    }

    #[inline(always)]
    fn bytes_required_when_copied_array(object: ObjectReference, rvm_type: Address) -> usize {
        trace!("VMObjectModel.bytes_required_when_copied_array");
        let mut size = {
            let num_elements = Self::get_array_length(object);
            unsafe {
                let log_element_size = (rvm_type + LOG_ELEMENT_SIZE_FIELD_OFFSET)
                    .load::<usize>();
                trace!("log_element_size={}", log_element_size);
                ARRAY_HEADER_SIZE + (num_elements << log_element_size)
            }
        };

        if ADDRESS_BASED_HASHING {
            let hash_state = unsafe {
                (object.to_address() + STATUS_OFFSET).load::<usize>()
                    & HASH_STATE_MASK
            };
            if hash_state != HASH_STATE_UNHASHED {
                size += HASHCODE_BYTES;
            }
        }

        unsafe { Address::from_usize(size).align_up(BYTES_IN_INT).as_usize() }
    }

    #[inline(always)]
    fn bytes_used(object: ObjectReference, rvm_type: Address) -> usize {
        trace!("VMObjectModel.bytes_used");
        unsafe {
            let is_class = (rvm_type + IS_CLASS_TYPE_FIELD_OFFSET).load::<bool>();
            let mut size = if is_class {
                (rvm_type + INSTANCE_SIZE_FIELD_OFFSET).load::<usize>()
            } else {
                let num_elements = Self::get_array_length(object);
                ARRAY_HEADER_SIZE
                    + (num_elements << (rvm_type + LOG_ELEMENT_SIZE_FIELD_OFFSET).load::<usize>())
            };

            if MOVES_OBJECTS {
                if ADDRESS_BASED_HASHING {
                    let hash_state = (object.to_address() + STATUS_OFFSET).load::<usize>()
                        & HASH_STATE_MASK;
                    if hash_state == HASH_STATE_HASHED_AND_MOVED {
                        size += HASHCODE_BYTES;
                    }
                }
            }

            if is_class {
                size
            } else {
                Address::from_usize(size).align_up(BYTES_IN_INT).as_usize()
            }
        }
    }

    #[inline(always)]
    fn move_object(immut_to_address: Address, from_obj: ObjectReference, immut_to_obj: ObjectReference,
                   num_bytes: usize, rvm_type: Address) -> ObjectReference {
        trace!("VMObjectModel.move_object");
        let mut to_address = immut_to_address;
        let mut to_obj = immut_to_obj;
        debug_assert!(to_address.is_zero() || to_obj.to_address().is_zero());

        // Default values
        let mut copy_bytes = num_bytes;
        let mut obj_ref_offset = OBJECT_REF_OFFSET;
        let mut status_word: usize = 0;
        let mut hash_state = HASH_STATE_UNHASHED;

        if ADDRESS_BASED_HASHING {
            unsafe {
                // Read the hash state (used below)
                status_word = (from_obj.to_address() + STATUS_OFFSET).load::<usize>();
                hash_state = status_word & HASH_STATE_MASK;
                if hash_state == HASH_STATE_HASHED {
                    // We do not copy the hashcode, but we do allocate it
                    copy_bytes -= HASHCODE_BYTES;

                    if !DYNAMIC_HASH_OFFSET {
                        // The hashcode is the first word, so we copy to object one word higher
                        if to_obj.to_address().is_zero() {
                            to_address += HASHCODE_BYTES;
                        }
                    }
                } else if !DYNAMIC_HASH_OFFSET && hash_state == HASH_STATE_HASHED_AND_MOVED {
                    // Simple operation (no hash state change), but one word larger header
                    obj_ref_offset += (HASHCODE_BYTES as isize);
                }
            }
        }

        if !to_obj.to_address().is_zero() {
            to_address = to_obj.to_address() + (-obj_ref_offset);
        }

        // Low memory word of source object
        let from_address = from_obj.to_address() + (-obj_ref_offset);

        // Do the copy
        unsafe { Self::aligned_32_copy(to_address, from_address, copy_bytes); }

        if to_obj.to_address().is_zero() {
            to_obj = unsafe { (to_address + obj_ref_offset).to_object_reference() };
        } else {
            debug_assert!(to_obj.to_address() == to_address + obj_ref_offset);
        }

        // Do we need to copy the hash code?
        if hash_state == HASH_STATE_HASHED {
            unsafe {
                let hash_code = from_obj.value() >> LOG_BYTES_IN_ADDRESS;
                if DYNAMIC_HASH_OFFSET {
                    (to_obj.to_address() + num_bytes + (-OBJECT_REF_OFFSET)
                        + (-(HASHCODE_BYTES as isize))).store::<usize>(hash_code);
                } else {
                    (to_obj.to_address() + HASHCODE_OFFSET)
                        .store::<usize>((hash_code << 1) | ALIGNMENT_MASK);
                }
                (to_obj.to_address() + STATUS_OFFSET).store::<usize>(status_word | HASH_STATE_HASHED_AND_MOVED);
                if HASH_STATS { HASH_TRANSITION2.fetch_add(1, Ordering::Relaxed); }
            }
        }

        to_obj
    }

    #[inline(always)]
    unsafe fn aligned_32_copy(dst: Address, src: Address, copy_bytes: usize) {
        trace!("VMObjectModel.aligned_32_copy");
        //debug_assert!(copy_bytes >= 0);
        debug_assert!(copy_bytes & BYTES_IN_INT - 1 == 0);
        debug_assert!(src.as_usize() & (BYTES_IN_INT - 1) == 0);
        debug_assert!(src.as_usize() & (BYTES_IN_INT - 1) == 0);
        debug_assert!(src + copy_bytes <= dst || src >= dst + BYTES_IN_INT);

        let cnt = copy_bytes;
        let src_end = src + cnt;
        let dst_end = dst + cnt;
        let overlap = !(src_end <= dst) && !(dst_end <= src);
        if overlap {
            memmove(dst.as_usize() as *mut c_void,
                    src.as_usize() as *mut c_void, cnt);
        } else {
            memcpy(dst.as_usize() as *mut c_void,
                   src.as_usize() as *mut c_void, cnt);
        }
    }

    #[inline(always)]
    fn get_alignment_array(rvm_type: Address) -> usize {
        trace!("VMObjectModel.get_alignment_array");
        unsafe { (rvm_type + RVM_ARRAY_ALIGNMENT_OFFSET).load::<usize>() }
    }

    #[inline(always)]
    fn get_alignment_class(rvm_type: Address) -> usize {
        trace!("VMObjectModel.get_alignment_class");
        if BYTES_IN_ADDRESS == BYTES_IN_DOUBLE {
            BYTES_IN_ADDRESS
        } else {
            unsafe { (rvm_type + RVM_CLASS_ALIGNMENT_OFFSET).load::<usize>() }
        }
    }

    #[inline(always)]
    fn get_offset_for_alignment_array(object: ObjectReference, rvm_type: Address) -> isize {
        trace!("VMObjectModel.get_offset_for_alignment_array");
        let mut offset = OBJECT_REF_OFFSET;

        if ADDRESS_BASED_HASHING && !DYNAMIC_HASH_OFFSET {
            let hash_state = unsafe {
                (object.to_address() + STATUS_OFFSET).load::<usize>() & HASH_STATE_MASK
            };
            if hash_state != HASH_STATE_UNHASHED {
                offset += HASHCODE_BYTES as isize;
            }
        }

        offset
    }

    #[inline(always)]
    fn get_offset_for_alignment_class(object: ObjectReference, rvm_type: Address) -> isize {
        trace!("VMObjectModel.get_offset_for_alignment_class");
        let mut offset = SCALAR_HEADER_SIZE as isize;

        if ADDRESS_BASED_HASHING && !DYNAMIC_HASH_OFFSET {
            let hash_state = unsafe {
                (object.to_address() + STATUS_OFFSET).load::<usize>() & HASH_STATE_MASK
            };
            if hash_state != HASH_STATE_UNHASHED {
                offset += HASHCODE_BYTES as isize;
            }
        }

        offset
    }
}
