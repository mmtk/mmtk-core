//! Data types for visiting metadata ranges at different granularities

use crate::util::Address;

/// The type for bit offset in a byte.
pub type BitOffset = u8;

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
    start_bit: BitOffset,
    end_addr: Address,
    end_bit: BitOffset,
    forwards: bool,
    visitor: &mut V,
) -> bool
where
    V: FnMut(BitByteRange) -> bool,
{
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
            bit_start: start_bit,
            bit_end: end_bit,
        });
    }

    // If the end is the 0th bit of the next byte of the start,
    // visit the bit range from the start to the end (bit 8) of the same byte.
    if start_addr + 1usize == end_addr && end_bit == 0 {
        return visitor(BitByteRange::BitsInByte {
            addr: start_addr,
            bit_start: start_bit,
            bit_end: 8_u8,
        });
    }

    // Otherwise, the range spans over multiple bytes, and is bit-unaligned at either the start or
    // the end.  Try to break it into (at most) three sub-ranges.

    let start_aligned = start_bit == 0;
    let end_aligned = end_bit == 0;

    // We cannot let multiple closures capture `visitor` mutably at the same time, so we pass the
    // visitor in as `v` every time.

    // visit bits within the first byte
    let visit_start = |v: &mut V| {
        if !start_aligned {
            v(BitByteRange::BitsInByte {
                addr: start_addr,
                bit_start: start_bit,
                bit_end: 8_u8,
            })
        } else {
            // The start is already aligned.  No sub-byte range at the start.
            false
        }
    };

    // visit whole bytes in the middle
    let visit_middle = |v: &mut V| {
        let start = if start_aligned {
            start_addr
        } else {
            // If the start is not aligned, the whole-byte range starts after the first byte.
            start_addr + 1usize
        };
        let end = end_addr;
        if start < end {
            v(BitByteRange::Bytes { start, end })
        } else {
            // There are no whole bytes in the middle.
            false
        }
    };

    // visit bits within the last byte
    let visit_end = |v: &mut V| {
        if !end_aligned {
            v(BitByteRange::BitsInByte {
                addr: end_addr,
                bit_start: 0_u8,
                bit_end: end_bit,
            })
        } else {
            // The end is aligned.  No sub-byte range at the end.
            false
        }
    };

    // Update each segments.
    if forwards {
        visit_start(visitor) || visit_middle(visitor) || visit_end(visitor)
    } else {
        visit_end(visitor) || visit_middle(visitor) || visit_start(visitor)
    }
}

#[cfg(test)]
mod tests {
    use crate::util::constants::BITS_IN_BYTE;

    use super::*;

    fn mk_addr(addr: usize) -> Address {
        unsafe { Address::from_usize(addr) }
    }

    fn break_bit_range_wrapped(
        start_addr: Address,
        start_bit: usize,
        end_addr: Address,
        end_bit: usize,
    ) -> Vec<BitByteRange> {
        let mut vec = vec![];
        break_bit_range(
            start_addr,
            start_bit as u8,
            end_addr,
            end_bit as u8,
            true,
            &mut |range| {
                vec.push(range);
                false
            },
        );
        vec
    }

    #[test]
    fn test_empty_range() {
        let base = mk_addr(0x1000);
        for bit in 0..BITS_IN_BYTE {
            let result = break_bit_range_wrapped(base, bit, base, bit);
            assert!(
                result.is_empty(),
                "Not empty. bit: {bit}, result: {result:?}"
            );
        }
    }

    #[test]
    fn test_subbyte_range() {
        let base = mk_addr(0x1000);
        for bit0 in 0..BITS_IN_BYTE {
            for bit1 in (bit0 + 1)..BITS_IN_BYTE {
                let result = break_bit_range_wrapped(base, bit0, base, bit1);
                assert_eq!(
                    result,
                    vec![BitByteRange::BitsInByte {
                        addr: base,
                        bit_start: bit0 as u8,
                        bit_end: bit1 as u8
                    }],
                    "Not equal.  bit0: {bit0}, bit1: {bit1}",
                );
            }
        }
    }

    #[test]
    fn test_end_byte_range() {
        let base = mk_addr(0x1000);
        for bit0 in 1..BITS_IN_BYTE {
            let result = break_bit_range_wrapped(base, bit0, base + 1usize, 0);
            assert_eq!(
                result,
                vec![BitByteRange::BitsInByte {
                    addr: base,
                    bit_start: bit0 as u8,
                    bit_end: BITS_IN_BYTE as u8
                }],
                "Not equal.  bit0: {bit0}",
            );
        }
    }

    #[test]
    fn test_adjacent_grain_range() {
        let base = mk_addr(0x1000);
        for bit0 in 1..BITS_IN_BYTE {
            for bit1 in 1..BITS_IN_BYTE {
                let result = break_bit_range_wrapped(base, bit0, base + 1usize, bit1);
                assert_eq!(
                    result,
                    vec![
                        BitByteRange::BitsInByte {
                            addr: base,
                            bit_start: bit0 as u8,
                            bit_end: BITS_IN_BYTE as u8,
                        },
                        BitByteRange::BitsInByte {
                            addr: base + 1usize,
                            bit_start: 0,
                            bit_end: bit1 as u8,
                        },
                    ],
                    "Not equal.  bit0: {bit0}, bit1: {bit1}",
                );
            }
        }
    }

    #[test]
    fn test_left_and_whole_range() {
        let base = mk_addr(0x1000);
        for bit0 in 1..BITS_IN_BYTE {
            for byte1 in 2usize..8 {
                let result = break_bit_range_wrapped(base, bit0, base + byte1, 0);
                assert_eq!(
                    result,
                    vec![
                        BitByteRange::BitsInByte {
                            addr: base,
                            bit_start: bit0 as u8,
                            bit_end: BITS_IN_BYTE as u8,
                        },
                        BitByteRange::Bytes {
                            start: base + 1usize,
                            end: base + byte1,
                        },
                    ],
                    "Not equal.  bit0: {bit0}, byte1: {byte1}",
                );
            }
        }
    }

    #[test]
    fn test_whole_and_right_range() {
        let base = mk_addr(0x1000);
        for byte0 in 1..8 {
            for bit1 in 1..BITS_IN_BYTE {
                let result = break_bit_range_wrapped(base - byte0, 0, base, bit1);
                assert_eq!(
                    result,
                    vec![
                        BitByteRange::Bytes {
                            start: base - byte0,
                            end: base,
                        },
                        BitByteRange::BitsInByte {
                            addr: base,
                            bit_start: 0,
                            bit_end: bit1 as u8,
                        },
                    ],
                    "Not equal.  byte0: {byte0}, bit1: {bit1}",
                );
            }
        }
    }

    #[test]
    fn test_whole_range() {
        let base = mk_addr(0x1000);
        let result = break_bit_range_wrapped(base, 0, base + 42usize, 0);
        assert_eq!(
            result,
            vec![BitByteRange::Bytes {
                start: base,
                end: base + 42usize,
            },],
        );
    }

    #[test]
    fn test_left_whole_right_range() {
        let base0 = mk_addr(0x1000);
        let base1 = mk_addr(0x2000);

        for bit0 in 1..BITS_IN_BYTE {
            for bit1 in 1..BITS_IN_BYTE {
                let result = break_bit_range_wrapped(base0 - 1usize, bit0, base1, bit1);
                assert_eq!(
                    result,
                    vec![
                        BitByteRange::BitsInByte {
                            addr: base0 - 1usize,
                            bit_start: bit0 as u8,
                            bit_end: BITS_IN_BYTE as u8,
                        },
                        BitByteRange::Bytes {
                            start: base0,
                            end: base1,
                        },
                        BitByteRange::BitsInByte {
                            addr: base1,
                            bit_start: 0,
                            bit_end: bit1 as u8,
                        },
                    ],
                    "Not equal.  bit0: {bit0}, bit1: {bit1}",
                );
            }
        }
    }
}
