pub const LOG_BITS_IN_BYTE: usize = 3;

#[cfg(target_pointer_width = "32")]
pub const LOG_BYTES_IN_ADDRESS: usize = 2;
#[cfg(target_pointer_width = "64")]
pub const LOG_BYTES_IN_ADDRESS: usize = 3;
pub const BYTES_IN_ADDRESS: usize = 1 << LOG_BYTES_IN_ADDRESS;
pub const LOG_BITS_IN_ADDRESS: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_ADDRESS;
pub const BITS_IN_ADDRESS: usize = 1 << LOG_BITS_IN_ADDRESS;

#[cfg(target_pointer_width = "32")]
pub const LOG_BYTES_IN_WORD: usize = 2;
#[cfg(target_pointer_width = "64")]
pub const LOG_BYTES_IN_WORD: usize = 3;
pub const BYTES_IN_WORD: usize = 1 << LOG_BYTES_IN_WORD;
pub const LOG_BITS_IN_WORD: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_WORD;
pub const BITS_IN_WORD: usize = 1 << LOG_BITS_IN_WORD;

#[cfg(target_pointer_width = "32")]
pub const LOG_BYTES_IN_EXTENT: usize = 2;
#[cfg(target_pointer_width = "64")]
pub const LOG_BYTES_IN_EXTENT: usize = 3;
pub const BYTES_IN_EXTENT: usize = 1 << LOG_BYTES_IN_EXTENT;
pub const LOG_BITS_IN_EXTENT: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_EXTENT;
pub const BITS_IN_EXTENT: usize = 1 << LOG_BITS_IN_EXTENT;

#[cfg(target_pointer_width = "32")]
pub const LOG_BYTES_IN_OFFSET: usize = 2;
#[cfg(target_pointer_width = "64")]
pub const LOG_BYTES_IN_OFFSET: usize = 3;
pub const BYTES_IN_OFFSET: usize = 1 << LOG_BYTES_IN_OFFSET;
pub const LOG_BITS_IN_OFFSET: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_OFFSET;
pub const BITS_IN_OFFSET: usize = 1 << LOG_BITS_IN_OFFSET;
