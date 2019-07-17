use util::Address;
use util::heap::layout::vm_layout_constants;
use util::heap::layout::heap_parameters;
use super::vmrequest::HEAP_LAYOUT_64BIT;
use util::constants::*;
use std::sync::atomic::{AtomicUsize, Ordering};


const TYPE_BITS: usize = 2;
const TYPE_SHARED: usize = 0;
const TYPE_CONTIGUOUS: usize = 1;
const TYPE_CONTIGUOUS_HI: usize = 3;
const TYPE_MASK: usize = (1 << TYPE_BITS) - 1;
const SIZE_SHIFT: usize = TYPE_BITS;
const SIZE_BITS: usize = 10;
const SIZE_MASK: usize = ((1 << SIZE_BITS) - 1) << SIZE_SHIFT;
const EXPONENT_SHIFT: usize = SIZE_SHIFT + SIZE_BITS;
const EXPONENT_BITS: usize = 5;
const EXPONENT_MASK: usize = ((1 << EXPONENT_BITS) - 1) << EXPONENT_SHIFT;
const MANTISSA_SHIFT: usize = EXPONENT_SHIFT + EXPONENT_BITS;
const MANTISSA_BITS: usize = 14;
const BASE_EXPONENT: usize = BITS_IN_INT - MANTISSA_BITS;

  /* 64-bit */
const INDEX_MASK: usize = !TYPE_MASK;
const INDEX_SHIFT: usize = TYPE_BITS;

lazy_static! {
    static ref DISCONTIGUOUS_SPACE_INDEX: AtomicUsize = AtomicUsize::default();
}

  // private static int discontiguousSpaceIndex = 0;
const DISCONTIG_INDEX_INCREMENT: usize = 1 << TYPE_BITS;

pub fn create_descriptor_from_heap_range(start: Address, end: Address) -> usize {
    let top = end == vm_layout_constants::HEAP_END;
    if HEAP_LAYOUT_64BIT {
        let space_index = if start > vm_layout_constants::HEAP_END { ::std::usize::MAX } else { start.0 >> vm_layout_constants::SPACE_SHIFT_64 };
        return space_index << INDEX_SHIFT |
            (if top { TYPE_CONTIGUOUS_HI } else { TYPE_CONTIGUOUS });
    }
    let chunks = (end - start) >> vm_layout_constants::LOG_BYTES_IN_CHUNK;
    if cfg!(debug) {
      // if (!start.isZero() && (chunks <= 0 || chunks >= (1 << SIZE_BITS))) {
      //   Log.write("SpaceDescriptor.createDescriptor(", start);
      //   Log.write(",", end);
      //   Log.writeln(")");
      //   Log.writeln("chunks = ", chunks);
      // }
        debug_assert!(!start.is_zero() && chunks > 0 && chunks < (1 << SIZE_BITS));
    }
    let mut tmp = start.0;
    tmp = tmp >> BASE_EXPONENT;
    let mut exponent = 0;
    while (tmp != 0) && ((tmp & 1) == 0) {
        tmp = tmp >> 1;
        exponent += 1;
    }
    let mantissa = tmp;
    debug_assert!((tmp << (BASE_EXPONENT + exponent)) == start.0);
    return (mantissa << MANTISSA_SHIFT) |
           (exponent << EXPONENT_SHIFT) |
           (chunks << SIZE_SHIFT) |
           (if top { TYPE_CONTIGUOUS_HI } else { TYPE_CONTIGUOUS });
}

pub fn create_descriptor() -> usize {
    DISCONTIGUOUS_SPACE_INDEX.store(DISCONTIGUOUS_SPACE_INDEX.load(Ordering::Relaxed) + DISCONTIG_INDEX_INCREMENT, Ordering::Relaxed);
    debug_assert!((DISCONTIGUOUS_SPACE_INDEX.load(Ordering::Relaxed) & TYPE_CONTIGUOUS) != TYPE_CONTIGUOUS);
    return DISCONTIGUOUS_SPACE_INDEX.load(Ordering::Relaxed);
}

pub fn is_contiguous(descriptor: usize) -> bool {
    ((descriptor & TYPE_CONTIGUOUS) == TYPE_CONTIGUOUS)
}

pub fn is_contiguous_hi(descriptor: usize) -> bool {
    ((descriptor & TYPE_MASK) == TYPE_CONTIGUOUS_HI)
}

#[allow(exceeding_bitshifts)]
pub fn get_start(descriptor: usize) -> Address {
    if cfg!(target_pointer_width = "64") {
      return unsafe { Address::from_usize(get_index(descriptor) << heap_parameters::LOG_SPACE_SIZE_64) };
    }
    debug_assert!(is_contiguous(descriptor));
    let mantissa = descriptor >> MANTISSA_SHIFT;
    let exponent = (descriptor & EXPONENT_MASK) >> EXPONENT_SHIFT;
    unsafe { Address::from_usize(mantissa << (BASE_EXPONENT + exponent)) }
}

pub fn get_extent(descriptor: usize) -> usize {
    if HEAP_LAYOUT_64BIT {
      return vm_layout_constants::SPACE_SIZE_64;
    }
    debug_assert!(is_contiguous(descriptor));
    let chunks = (descriptor & SIZE_MASK) >> SIZE_SHIFT;
    let size = chunks << vm_layout_constants::LOG_BYTES_IN_CHUNK;
    return size;
}

pub fn get_index(descriptor: usize) -> usize {
    debug_assert!(HEAP_LAYOUT_64BIT);
    return (descriptor & INDEX_MASK) >> INDEX_SHIFT;
}