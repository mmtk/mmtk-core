use super::memory_manager_constants;
use super::memory_manager_constants::*;
use super::super::unboxed_size_constants::*;
use super::java_size_constants::*;
use super::misc_header_constants::*;

/** Number of bytes in object's TIB pointer */
pub const TIB_BYTES: usize = BYTES_IN_ADDRESS;
/** Number of bytes indicating an object's status */
pub const STATUS_BYTES: usize = BYTES_IN_ADDRESS;

pub const ALIGNMENT_MASK: usize = 0x00000001;
pub const ALIGNMENT_VALUE: usize = 0xdeadbeef;
pub const LOG_MIN_ALIGNMENT: usize = LOG_BYTES_IN_INT;

/**
 * Number of bytes used to store the array length. We use 64 bits
 * for the length on a 64 bit architecture as this makes the other
 * words 8-byte aligned, and the header has to be 8-byte aligned.
 */
#[cfg(target_pointer_width = "32")]
pub const ARRAY_LENGTH_BYTES: usize = BYTES_IN_INT;
#[cfg(target_pointer_width = "64")]
pub const ARRAY_LENGTH_BYTES: usize = BYTES_IN_ADDRESS;

/** Number of bytes used by the Java Header */
pub const JAVA_HEADER_BYTES: usize = TIB_BYTES + STATUS_BYTES;
/** Number of bytes used by the GC Header */
pub const GC_HEADER_BYTES: usize = memory_manager_constants::GC_HEADER_BYTES;
/** Number of bytes used by the miscellaneous header */
pub const MISC_HEADER_BYTES: usize = NUM_BYTES_HEADER;
/** Size of GC and miscellaneous headers */
pub const OTHER_HEADER_BYTES: usize = GC_HEADER_BYTES + MISC_HEADER_BYTES;

/** Offset of array length from object reference */
pub const ARRAY_LENGTH_OFFSET: isize = - (ARRAY_LENGTH_BYTES as isize);
/** Offset of the first field from object reference */
pub const FIELD_ZERO_OFFSET: isize = ARRAY_LENGTH_OFFSET;
/** Offset of the Java header from the object reference */
pub const JAVA_HEADER_OFFSET: isize = ARRAY_LENGTH_OFFSET - (JAVA_HEADER_BYTES as isize);
/** Offset of the miscellaneous header from the object reference */
pub const MISC_HEADER_OFFSET: isize = JAVA_HEADER_OFFSET - (MISC_HEADER_BYTES as isize);
/** Offset of the garbage collection header from the object reference */
pub const GC_HEADER_OFFSET: isize = MISC_HEADER_OFFSET - (GC_HEADER_BYTES as isize);
/** Offset of first element of an array */
pub const ARRAY_BASE_OFFSET: isize = 0;

/**
 * This object model supports two schemes for hashcodes:
 * (1) a 10 bit hash code in the object header
 * (2) use the address of the object as its hashcode.
 *     In a copying collector, this forces us to add a word
 *     to copied objects that have had their hashcode taken.
 */
pub const ADDRESS_BASED_HASHING: bool = !GENERATE_GC_TRACE;

/** How many bits in the header are available for the GC and MISC headers? */
pub const NUM_AVAILABLE_BITS: usize = if_then_else_usize!(ADDRESS_BASED_HASHING, 8, 2);

/**
 * Does this object model use the same header word to contain
 * the TIB and a forwarding pointer?
 */
pub const FORWARDING_PTR_OVERLAYS_TIB: bool = false;

/**
 * Does this object model place the hash for a hashed and moved object
 * after the data (at a dynamic offset)
 */
pub const DYNAMIC_HASH_OFFSET: bool = ADDRESS_BASED_HASHING && NEEDS_LINEAR_SCAN;

/**
 * Can we perform a linear scan?
 */
pub const ALLOWS_LINEAR_SCAN: bool = true;

/**
 * Do we need to segregate arrays and scalars to do a linear scan?
 */
pub const SEGREGATE_ARRAYS_FOR_LINEAR_SCAN: bool = false;

/*
 * Stuff for address based hashing
 */
pub const HASH_STATE_UNHASHED: usize = 0;
pub const HASH_STATE_HASHED: usize = 1 << 8; //0x00000100
pub const HASH_STATE_HASHED_AND_MOVED: usize = 3 << 8; //0x0000300
pub const HASH_STATE_MASK: usize = (HASH_STATE_UNHASHED | HASH_STATE_HASHED)
        | HASH_STATE_HASHED_AND_MOVED;

pub const HASHCODE_BYTES: usize = BYTES_IN_INT;
pub const HASHCODE_OFFSET: isize = GC_HEADER_OFFSET - (HASHCODE_BYTES as isize);