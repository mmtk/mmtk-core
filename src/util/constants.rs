use crate::util::alloc::embedded_meta_data::LOG_BYTES_IN_REGION;

/****************************************************************************
 *
 * Generic sizes
 */

/// log2 of the number of bytes in a byte
pub const LOG_BYTES_IN_BYTE: u8 = 0;
/// The number of bytes in a byte
pub const BYTES_IN_BYTE: usize = 1;
/// log2 of the number of bits in a byte
pub const LOG_BITS_IN_BYTE: u8 = 3;
/// The number of bits in a byte
pub const BITS_IN_BYTE: usize = 1 << LOG_BITS_IN_BYTE;

/// log2 of the number of bytes in a gigabyte
pub const LOG_BYTES_IN_GBYTE: u8 = 30;
/// The number of bytes in a gigabyte
pub const BYTES_IN_GBYTE: usize = 1 << LOG_BYTES_IN_GBYTE;

/// log2 of the number of bytes in a megabyte
pub const LOG_BYTES_IN_MBYTE: u8 = 20;
/// The number of bytes in a megabyte
pub const BYTES_IN_MBYTE: usize = 1 << LOG_BYTES_IN_MBYTE;

/// log2 of the number of bytes in a kilobyte
pub const LOG_BYTES_IN_KBYTE: u8 = 10;
/// The number of bytes in a kilobyte
pub const BYTES_IN_KBYTE: usize = 1 << LOG_BYTES_IN_KBYTE;

/****************************************************************************
 *
 * Java-specific sizes currently required by MMTk
 *
 * TODO MMTk should really become independent of these Java types
 */

pub const LOG_BYTES_IN_CHAR: u8 = 1;
pub const BYTES_IN_CHAR: usize = 1 << LOG_BYTES_IN_CHAR;
pub const LOG_BITS_IN_CHAR: u8 = LOG_BITS_IN_BYTE + LOG_BYTES_IN_CHAR;
pub const BITS_IN_CHAR: usize = 1 << LOG_BITS_IN_CHAR;

pub const LOG_BYTES_IN_SHORT: u8 = 1;
pub const BYTES_IN_SHORT: usize = 1 << LOG_BYTES_IN_SHORT;
pub const LOG_BITS_IN_SHORT: u8 = LOG_BITS_IN_BYTE + LOG_BYTES_IN_SHORT;
pub const BITS_IN_SHORT: usize = 1 << LOG_BITS_IN_SHORT;

pub const LOG_BYTES_IN_INT: u8 = 2;
pub const BYTES_IN_INT: usize = 1 << LOG_BYTES_IN_INT;
pub const LOG_BITS_IN_INT: u8 = LOG_BITS_IN_BYTE + LOG_BYTES_IN_INT;
pub const BITS_IN_INT: usize = 1 << LOG_BITS_IN_INT;

pub const LOG_BYTES_IN_LONG: u8 = 3;
pub const BYTES_IN_LONG: usize = 1 << LOG_BYTES_IN_LONG;
pub const LOG_BITS_IN_LONG: u8 = LOG_BITS_IN_BYTE + LOG_BYTES_IN_LONG;
pub const BITS_IN_LONG: usize = 1 << LOG_BITS_IN_LONG;

pub const MAX_INT: usize = i32::max_value() as usize; // 0x7fff_ffff
pub const MIN_INT: usize = i32::min_value() as u32 as usize; // 0x8000_0000

/****************************************************************************
 *
 * VM-Specific sizes
 */

#[cfg(target_pointer_width = "32")]
pub const LOG_BYTES_IN_ADDRESS: u8 = 2;
#[cfg(target_pointer_width = "64")]
pub const LOG_BYTES_IN_ADDRESS: u8 = 3;
pub const BYTES_IN_ADDRESS: usize = 1 << LOG_BYTES_IN_ADDRESS;
pub const LOG_BITS_IN_ADDRESS: usize = LOG_BITS_IN_BYTE as usize + LOG_BYTES_IN_ADDRESS as usize;
pub const BITS_IN_ADDRESS: usize = 1 << LOG_BITS_IN_ADDRESS;

// Note that in MMTk we currently define WORD & ADDRESS to be the same size
pub const LOG_BYTES_IN_WORD: u8 = LOG_BYTES_IN_ADDRESS;
pub const BYTES_IN_WORD: usize = 1 << LOG_BYTES_IN_WORD;
pub const LOG_BITS_IN_WORD: usize = LOG_BITS_IN_BYTE as usize + LOG_BYTES_IN_WORD as usize;
pub const BITS_IN_WORD: usize = 1 << LOG_BITS_IN_WORD;

pub const LOG_BYTES_IN_PAGE: u8 = 12; // XXX: This is a lie
pub const BYTES_IN_PAGE: usize = 1 << LOG_BYTES_IN_PAGE;
pub const LOG_BITS_IN_PAGE: usize = LOG_BITS_IN_BYTE as usize + LOG_BYTES_IN_PAGE as usize;
pub const BITS_IN_PAGE: usize = 1 << LOG_BITS_IN_PAGE;

/* Assume byte-addressability */
pub const LOG_BYTES_IN_ADDRESS_SPACE: u8 = BITS_IN_ADDRESS as u8;

// TODO: Should this be VM specific?
pub const LOG_MIN_OBJECT_SIZE: u8 = LOG_BYTES_IN_WORD;
pub const MIN_OBJECT_SIZE: usize = 1 << LOG_MIN_OBJECT_SIZE;

/****************************************************************************
 *
 * Default options
 */

pub const DEFAULT_STRESS_FACTOR: usize = usize::max_value();
