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

/// Break a bit range into sub-ranges of whole bytes and in-byte bits.
///
/// This method is primarily used for iterating side metadata for a data address range. As we cannot
/// guarantee that the data address range can be mapped to whole metadata bytes, we have to deal
/// with visiting only a bit range in a metadata byte.
///
/// The bit range starts at the bit at index `meta_start_bit` in the byte at address
/// `meta_start_addr`, and ends at (but does not include) the bit at index `meta_end_bit` in the
/// byte at address `meta_end_addr`.
///
/// Arguments:
/// * `forwards`: If true, we iterate forwards (from start/low address to end/high address).
///               Otherwise, we iterate backwards (from end/high address to start/low address).
/// * `visitor`: The callback that visits ranges of bits or bytes.  It returns whether the itertion
///   is early terminated.
///
/// Returns true if we iterate through every bits in the range. Return false if we abort iteration
/// early.
pub fn break_bit_range<V>(
    start_addr: Address,
    start_bit: u8,
    end_addr: Address,
    end_bit: u8,
    forwards: bool,
    visitor: &mut V,
) -> bool
where
    V: FnMut(BitByteRange) -> bool,
{
    trace!(
        "iterate_meta_bits: {} {}, {} {}",
        start_addr,
        start_bit,
        end_addr,
        end_bit
    );

    // The start and the end are the same, we don't need to do anything.
    if start_addr == end_addr && start_bit == end_bit {
        return false;
    }

    // If the range is already byte-aligned, visit whole bits.
    if start_bit == 0 && end_bit == 0 {
        return visitor(BitByteRange::Bytes {
            start: start_addr,
            end: end_addr,
        });
    }

    // If the start and the end are within the same byte,
    // visit the bit range within the byte.
    if start_addr == end_addr {
        return visitor(BitByteRange::BitsInByte {
            addr: start_addr,
            bit_start: start_bit as usize,
            bit_end: end_bit as usize,
        });
    }

    // If the end is the 0th bit of the next byte of the start,
    // visit the bit range from the start to the end (bit 8) of the same byte.
    if start_addr + 1usize == end_addr && end_bit == 0 {
        return visitor(BitByteRange::BitsInByte {
            addr: start_addr,
            bit_start: start_bit as usize,
            bit_end: 8usize,
        });
    }

    // Otherwise, the range spans over multiple bytes, and is bit-unaligned at either the start
    // or the end.  Try to break it into (at most) three sub-ranges.

    // We cannot let multiple closures capture `visitor` mutably at the same time, so we
    // pass the visitor in as `v` every time.

    // update bits in the first byte
    let visit_start = |v: &mut V| {
        v(BitByteRange::BitsInByte {
            addr: start_addr,
            bit_start: start_bit as usize,
            bit_end: 8usize,
        })
    };

    // update bytes in the middle
    let visit_middle = |v: &mut V| {
        let start = start_addr + 1usize;
        let end = end_addr;
        if start < end {
            // non-empty middle range
            v(BitByteRange::Bytes { start, end })
        } else {
            // empty middle range
            false
        }
    };

    // update bits in the last byte
    let visit_end = |v: &mut V| {
        v(BitByteRange::BitsInByte {
            addr: end_addr,
            bit_start: 0usize,
            bit_end: end_bit as usize,
        })
    };

    // Update each segments.
    if forwards {
        visit_start(visitor) || visit_middle(visitor) || visit_end(visitor)
    } else {
        visit_end(visitor) || visit_middle(visitor) || visit_start(visitor)
    }
}
