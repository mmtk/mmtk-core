use std::sync::atomic::AtomicUsize;

use super::java_header_constants;
use super::java_header_constants::*;

pub const SCALAR_HEADER_SIZE: usize = JAVA_HEADER_BYTES + OTHER_HEADER_BYTES;
pub const ARRAY_HEADER_SIZE: usize = SCALAR_HEADER_SIZE + ARRAY_LENGTH_BYTES;

/** offset of object reference from the lowest memory word */
pub const OBJECT_REF_OFFSET: usize = ARRAY_HEADER_SIZE;  // from start to ref
pub const TIB_OFFSET: isize = JAVA_HEADER_OFFSET;
pub const STATUS_OFFSET: isize = TIB_OFFSET + STATUS_BYTES as isize;
#[cfg(target_endian = "little")]
pub const AVAILABLE_BITS_OFFSET: isize = STATUS_OFFSET;
#[cfg(target_endian = "big")]
pub const AVAILABLE_BITS_OFFSET: isize = STATUS_OFFSET + STATUS_BYTES - 1;

/*
 * Used for 10 bit header hash code in header (!ADDRESS_BASED_HASHING)
 */
pub const HASH_CODE_SHIFT: usize = 2;
pub const HASH_CODE_MASK: usize = ((1 << 10) - 1) << HASH_CODE_SHIFT;
pub static HASH_CODE_GENERATOR: AtomicUsize = AtomicUsize::new(0); // seed for generating hash codes with copying collectors.

/** How many bits are allocated to a thin lock? */
pub const NUM_THIN_LOCK_BITS: usize = if_then_else_usize!(ADDRESS_BASED_HASHING, 22, 20);
/** How many bits to shift to get the thin lock? */
pub const THIN_LOCK_SHIFT: usize = if_then_else_usize!(ADDRESS_BASED_HASHING, 10, 12);
/** How many bytes do we have to offset to get to the high locking bits */
#[cfg(target_endian = "little")]
pub const THIN_LOCK_DEDICATED_U16_OFFSET: usize = 2;
#[cfg(all(target_endian = "big", target_pointer_width = "64"))]
pub const THIN_LOCK_DEDICATED_U16_OFFSET: usize = 4;
#[cfg(all(target_endian = "big", target_pointer_width = "32"))]
pub const THIN_LOCK_DEDICATED_U16_OFFSET: usize = 0;
/** How many bits do we have to shift to only hold the high locking bits */
pub const THIN_LOCK_DEDICATED_U16_SHIFT: usize  = 16;

/** The alignment value **/
pub const ALIGNMENT_VALUE: usize = java_header_constants::ALIGNMENT_VALUE;
pub const LOG_MIN_ALIGNMENT: usize = java_header_constants::LOG_MIN_ALIGNMENT;