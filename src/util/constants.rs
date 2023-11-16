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

// Java-specific sizes currently used by MMTk
// TODO: MMTk should really become independent of these Java types: https://github.com/mmtk/mmtk-core/issues/922
mod java_specific_constants {
    use super::LOG_BITS_IN_BYTE;

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
}
pub(crate) use java_specific_constants::*;

#[cfg(target_pointer_width = "32")]
/// log2 of the number of bytes in an address
pub const LOG_BYTES_IN_ADDRESS: u8 = 2;
#[cfg(target_pointer_width = "64")]
/// log2 of the number of bytes in an address
pub const LOG_BYTES_IN_ADDRESS: u8 = 3;
/// The number of bytes in an address
pub const BYTES_IN_ADDRESS: usize = 1 << LOG_BYTES_IN_ADDRESS;
/// log2 of the number of bits in an address
pub const LOG_BITS_IN_ADDRESS: usize = LOG_BITS_IN_BYTE as usize + LOG_BYTES_IN_ADDRESS as usize;
/// The number of bits in an address
pub const BITS_IN_ADDRESS: usize = 1 << LOG_BITS_IN_ADDRESS;

/// log2 of the number of bytes in a word
pub const LOG_BYTES_IN_WORD: u8 = LOG_BYTES_IN_ADDRESS;
/// The number of bytes in a word
pub const BYTES_IN_WORD: usize = 1 << LOG_BYTES_IN_WORD;
/// log2 of the number of bits in a word
pub const LOG_BITS_IN_WORD: usize = LOG_BITS_IN_BYTE as usize + LOG_BYTES_IN_WORD as usize;
/// The number of bits in a word
pub const BITS_IN_WORD: usize = 1 << LOG_BITS_IN_WORD;

/// log2 of the number of bytes in a page
pub const LOG_BYTES_IN_PAGE: u8 = 12;
/// The number of bytes in a page
pub const BYTES_IN_PAGE: usize = 1 << LOG_BYTES_IN_PAGE;
/// log2 of the number of bits in a page
pub const LOG_BITS_IN_PAGE: usize = LOG_BITS_IN_BYTE as usize + LOG_BYTES_IN_PAGE as usize;
/// The number of bits in a page
pub const BITS_IN_PAGE: usize = 1 << LOG_BITS_IN_PAGE;

/// log2 of the number of bytes in the address space
pub const LOG_BYTES_IN_ADDRESS_SPACE: u8 = BITS_IN_ADDRESS as u8;

/// log2 of the minimal object size in bytes.
// TODO: this should be VM specific.
pub const LOG_MIN_OBJECT_SIZE: u8 = LOG_BYTES_IN_WORD;
/// The minimal object size in bytes
pub const MIN_OBJECT_SIZE: usize = 1 << LOG_MIN_OBJECT_SIZE;
