use crate::util::alloc::embedded_meta_data::LOG_BYTES_IN_REGION;

/**
 * Modes.
 */
pub const INSTANCE_FIELD: usize = 0;
pub const ARRAY_ELEMENT: usize = 1;

/****************************************************************************
 *
 * Generic sizes
 */

pub const LOG_BYTES_IN_BYTE: u8 = 0;
pub const BYTES_IN_BYTE: usize = 1;
pub const LOG_BITS_IN_BYTE: u8 = 3;
pub const BITS_IN_BYTE: usize = 1 << LOG_BITS_IN_BYTE;

pub const LOG_BYTES_IN_GBYTE: u8 = 30;
pub const BYTES_IN_GBYTE: usize = 1 << LOG_BYTES_IN_GBYTE;

pub const LOG_BYTES_IN_MBYTE: u8 = 20;
pub const BYTES_IN_MBYTE: usize = 1 << LOG_BYTES_IN_MBYTE;

pub const LOG_BYTES_IN_KBYTE: u8 = 10;
pub const BYTES_IN_KBYTE: usize = 1 << LOG_BYTES_IN_KBYTE;

/****************************************************************************
 *
 * Card scanning
 */

pub const SUPPORT_CARD_SCANNING: bool = false;
pub const LOG_CARD_META_SIZE: usize = 2; // each card consumes four bytes of metadata
pub const LOG_CARD_UNITS: usize = 10; // number of units tracked per card
pub const LOG_CARD_GRAIN: usize = 0; // track at byte grain, save shifting
pub const LOG_CARD_BYTES: usize = LOG_CARD_UNITS + LOG_CARD_GRAIN;
pub const LOG_CARD_META_BYTES: usize = LOG_BYTES_IN_REGION - LOG_CARD_BYTES + LOG_CARD_META_SIZE;
pub const LOG_CARD_META_PAGES: usize = LOG_CARD_META_BYTES - LOG_BYTES_IN_PAGE as usize;
/// FIXME: Card scanning is not supported at the moment. Move this to side-metadata in the future.
pub const CARD_META_PAGES_PER_REGION: usize = if SUPPORT_CARD_SCANNING {
    1 << LOG_CARD_META_PAGES
} else {
    0
};
pub const CARD_MASK: usize = (1 << LOG_CARD_BYTES) - 1;

/**
 * Lazy sweeping - controlled from here because PlanConstraints needs to
 * tell the VM that we need to support linear scan.
 */
pub const LAZY_SWEEP: bool = true;

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
