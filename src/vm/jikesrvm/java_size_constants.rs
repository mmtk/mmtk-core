pub const LOG_BYTES_IN_BYTE: usize = 0;
pub const BYTES_IN_BYTE: usize = 1;
pub const LOG_BITS_IN_BYTE: usize = 3;
pub const BITS_IN_BYTE: usize = 1 << LOG_BITS_IN_BYTE;

pub const LOG_BYTES_IN_BOOLEAN: usize = 0;
pub const BYTES_IN_BOOLEAN: usize = 1 << LOG_BYTES_IN_BOOLEAN;
pub const LOG_BITS_IN_BOOLEAN: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_BOOLEAN;
pub const BITS_IN_BOOLEAN: usize = 1 << LOG_BITS_IN_BOOLEAN;

pub const LOG_BYTES_IN_CHAR: usize = 1;
pub const BYTES_IN_CHAR: usize = 1 << LOG_BYTES_IN_CHAR;
pub const LOG_BITS_IN_CHAR: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_CHAR;
pub const BITS_IN_CHAR: usize = 1 << LOG_BITS_IN_CHAR;

pub const LOG_BYTES_IN_SHORT: usize = 1;
pub const BYTES_IN_SHORT: usize = 1 << LOG_BYTES_IN_SHORT;
pub const LOG_BITS_IN_SHORT: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_SHORT;
pub const BITS_IN_SHORT: usize = 1 << LOG_BITS_IN_SHORT;

pub const LOG_BYTES_IN_INT: usize = 2;
pub const BYTES_IN_INT: usize = 1 << LOG_BYTES_IN_INT;
pub const LOG_BITS_IN_INT: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_INT;
pub const BITS_IN_INT: usize = 1 << LOG_BITS_IN_INT;

pub const LOG_BYTES_IN_FLOAT: usize = 2;
pub const BYTES_IN_FLOAT: usize = 1 << LOG_BYTES_IN_FLOAT;
pub const LOG_BITS_IN_FLOAT: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_FLOAT;
pub const BITS_IN_FLOAT: usize = 1 << LOG_BITS_IN_FLOAT;

pub const LOG_BYTES_IN_LONG: usize = 3;
pub const BYTES_IN_LONG: usize = 1 << LOG_BYTES_IN_LONG;
pub const LOG_BITS_IN_LONG: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_LONG;
pub const BITS_IN_LONG: usize = 1 << LOG_BITS_IN_LONG;

pub const LOG_BYTES_IN_DOUBLE: usize = 3;
pub const BYTES_IN_DOUBLE: usize = 1 << LOG_BYTES_IN_DOUBLE;
pub const LOG_BITS_IN_DOUBLE: usize = LOG_BITS_IN_BYTE + LOG_BYTES_IN_DOUBLE;
pub const BITS_IN_DOUBLE: usize = 1 << LOG_BITS_IN_DOUBLE;