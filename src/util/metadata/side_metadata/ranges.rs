//! Data types for visiting metadata ranges at different granularities

use crate::util::Address;

/// The type for bit offset in a byte, word or a SIMD vector.
///
/// We use usize because it is generic and we may use AVX-512 some day, where u8 (256 max) is not
/// big enough.
pub type BitOffset = usize;

/// A range of bytes or bits within a byte.  It is the unit of visiting a contiguous bit range of a
/// side metadata.
///
/// In general, a bit range of a bitmap starts with multiple bits in the byte, followed by many
/// whole bytes, and ends with multiple bits in the last byte.
///
/// A range is never empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitByteRange {
    /// A range of whole bytes.
    Bytes {
        /// The starting address (inclusive) of the bytes.
        start: Address,
        /// The ending address (exclusive) of the bytes.
        end: Address,
    },
    /// A range of bits within a byte.
    BitsInByte {
        /// The address of the byte.
        addr: Address,
        /// The starting bit index (inclusive), starting with zero from the low-order bit.
        bit_start: BitOffset,
        /// The ending bit index (exclusive),  starting with zero from the low-order bit.  This may
        /// be 8 which means the range includes the highest bit.  Be careful when shifting a `u8`
        /// value because shifting an `u8` by 8 is considered an overflow in Rust.
        bit_end: BitOffset,
    },
}
