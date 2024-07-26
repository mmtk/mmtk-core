//! Mechanisms for breaking a range in a bitmap into whole-grain and sub-grain ranges to maximize
//! bulk bitmap access.
//!
//! In this module, the term "grain" refers to the unit at which we access a bitmap. It can be a
//! byte (u8), a word (usize) or other power-of-two byte sizes.
//!
//! The `std::simd` module is still a nightly feature as of Rust 1.79.  When it stablizes, we can
//! allow the granularity to be a SIMD vector, too.

use num_traits::PrimInt;

use crate::util::{constants::LOG_BITS_IN_BYTE, Address};

/// Offset of bits in a grain.
type BitOffset = usize;

#[derive(Debug, Clone, Copy, PartialOrd, Ord, PartialEq, Eq)]
pub struct Granularity {
    log_bytes: usize,
}

impl Granularity {
    pub const fn of_log_bytes(log_bytes: usize) -> Self {
        Self { log_bytes }
    }

    pub const fn of_log_bits(log_bits: usize) -> Self {
        Self {
            log_bytes: log_bits >> LOG_BITS_IN_BYTE,
        }
    }

    pub const fn of_type<T: PrimInt>() -> Self {
        let bytes = std::mem::size_of::<T>();
        let log_bytes = bytes.ilog2() as usize;
        Self::of_log_bytes(log_bytes)
    }

    pub const fn log_bytes(&self) -> usize {
        self.log_bytes
    }

    pub const fn log_bits(&self) -> usize {
        self.log_bytes() + LOG_BITS_IN_BYTE as usize
    }

    pub const fn bytes(&self) -> usize {
        1 << self.log_bytes()
    }

    pub const fn bits(&self) -> usize {
        1 << self.log_bits()
    }
}

/// Bit address within a grain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitAddress {
    pub addr: Address,
    pub bit: BitOffset,
}

impl BitAddress {
    pub fn is_grain_aligned(&self, granularity: Granularity) -> bool {
        self.bit == 0 && self.addr.is_aligned_to(granularity.bytes())
    }

    pub fn is_normalized(&self, granularity: Granularity) -> bool {
        self.addr.is_aligned_to(granularity.bytes()) && self.bit < granularity.bits()
    }

    pub fn normalize(&self, granularity: Granularity) -> Self {
        // Transfer unaligned bytes from addr to bits
        let aligned_addr = self.addr.align_down(granularity.bytes());
        let rem_bytes = self.addr - aligned_addr;
        let rem_bits = self.bit + (rem_bytes << LOG_BITS_IN_BYTE);

        // Transfer bits outside granularity back to addr
        let bit = rem_bits % granularity.bits();
        let carry_bits = rem_bits - bit;
        let addr = aligned_addr + (carry_bits >> LOG_BITS_IN_BYTE);

        Self { addr, bit }
    }

    pub fn align_down_to_grain(&self, granularity: Granularity) -> Address {
        debug_assert!(self.is_normalized(granularity));
        self.addr.align_down(granularity.bytes())
    }

    pub fn align_up_to_grain(&self, granularity: Granularity) -> Address {
        debug_assert!(self.is_normalized(granularity));
        if self.bit > 0 {
            (self.addr + 1usize).align_up(granularity.bytes())
        } else {
            self.addr.align_up(granularity.bytes())
        }
    }
}

pub trait AddressToBitAddress {
    fn with_bit_offset(&self, offset: BitOffset) -> BitAddress;
}

impl AddressToBitAddress for Address {
    fn with_bit_offset(&self, offset: BitOffset) -> BitAddress {
        BitAddress {
            addr: *self,
            bit: offset,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitRange {
    WholeGrain {
        start: Address,
        end: Address,
    },
    SubGrain {
        addr: Address,
        bit_start: BitOffset,
        bit_end: BitOffset,
    },
}

pub fn bit_mask<T: PrimInt>(granularity: Granularity, bit_start: usize, bit_end: usize) -> T {
    debug_assert!(
        bit_start < bit_end,
        "bit_start ({bit_start}) must be less than bit_end ({bit_end})"
    );
    debug_assert!(
        bit_end <= granularity.bits(),
        "Bit offset too high.  Granularity is {} bits, bit_end is {}",
        granularity.bits(),
        bit_end
    );

    if bit_end == granularity.bits() {
        !T::zero() << bit_start
    } else {
        (!T::zero() << bit_start) & !(!T::zero() << bit_end)
    }
}

pub fn break_range(
    granularity: Granularity,
    start: BitAddress,
    end: BitAddress,
) -> Vec<VisitRange> {
    debug_assert!(
        start.is_normalized(granularity),
        "{start:?} is not normalized for {granularity:?}"
    );
    debug_assert!(
        end.is_normalized(granularity),
        "{end:?} is not normalized for {granularity:?}"
    );

    warn!("break_range: {granularity:?} {start:?} {end:?}");

    if start.addr == end.addr {
        // The start and the end are in the same grain.
        if start.bit == end.bit {
            // Empty.
            return vec![];
        } else {
            // Yield only one SubGrain range.
            return vec![VisitRange::SubGrain {
                addr: start.addr,
                bit_start: start.bit,
                bit_end: end.bit,
            }];
        }
    }

    // Start with a sub-grain range if the start is not aligned.
    let start_subgrain = (!start.is_grain_aligned(granularity)).then(|| VisitRange::SubGrain {
        addr: start.addr,
        bit_start: start.bit,
        bit_end: granularity.bits(),
    });

    // Yield a whole-grain in the middle if long enough.
    let start_whole = start.align_up_to_grain(granularity);
    let end_whole = end.align_down_to_grain(granularity);
    let whole = (start_whole < end_whole).then(|| VisitRange::WholeGrain {
        start: start_whole,
        end: end_whole,
    });

    // Finally yield a sub-grain range in the end if not aligned.
    let end_subgrain = (!end.is_grain_aligned(granularity)).then(|| VisitRange::SubGrain {
        addr: end.addr,
        bit_start: 0,
        bit_end: end.bit,
    });

    [start_subgrain, whole, end_subgrain]
        .into_iter()
        .flatten()
        .collect()
}

pub fn break_range_callback<F>(
    granularity: Granularity,
    start: BitAddress,
    end: BitAddress,
    forwards: bool,
    visitor: &mut F,
) where
    F: FnMut(VisitRange) -> bool,
{
    debug_assert!(
        start.is_normalized(granularity),
        "{start:?} is not normalized for {granularity:?}"
    );
    debug_assert!(
        end.is_normalized(granularity),
        "{end:?} is not normalized for {granularity:?}"
    );

    warn!("break_range: {granularity:?} {start:?} {end:?}");

    if start.addr == end.addr {
        // The start and the end are in the same grain.
        if start.bit == end.bit {
            return;
        } else {
            // Yield only one SubGrain range.
            visitor(VisitRange::SubGrain {
                addr: start.addr,
                bit_start: start.bit,
                bit_end: end.bit,
            });
            return;
        }
    }

    // Start with a sub-grain range if the start is not aligned.
    let start_subgrain = |v: &mut F| {
        if !start.is_grain_aligned(granularity) {
            v(VisitRange::SubGrain {
                addr: start.addr,
                bit_start: start.bit,
                bit_end: granularity.bits(),
            });
        }
    };

    // Yield a whole-grain in the middle if long enough.
    let start_whole = start.align_up_to_grain(granularity);
    let end_whole = end.align_down_to_grain(granularity);
    let whole = |v: &mut F| {
        if start_whole < end_whole {
            v(VisitRange::WholeGrain {
                start: start_whole,
                end: end_whole,
            });
        }
    };

    // Finally yield a sub-grain range in the end if not aligned.
    let end_subgrain = |v: &mut F| {
        if !end.is_grain_aligned(granularity) {
            v(VisitRange::SubGrain {
                addr: end.addr,
                bit_start: 0,
                bit_end: end.bit,
            });
        }
    };

    if forwards {
        start_subgrain(visitor);
        whole(visitor);
        end_subgrain(visitor);
    } else {
        end_subgrain(visitor);
        whole(visitor);
        start_subgrain(visitor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_addr(addr: usize) -> Address {
        unsafe { Address::from_usize(addr) }
    }

    fn mk_ba(addr: usize, bit: usize) -> BitAddress {
        BitAddress {
            addr: mk_addr(addr),
            bit,
        }
    }

    const G8BIT: Granularity = Granularity::of_log_bytes(0);
    const G64BIT: Granularity = Granularity::of_log_bytes(3);
    const GS: [Granularity; 2] = [G8BIT, G64BIT];

    #[test]
    fn test_grain_alignment() {
        assert!(mk_ba(0x1000, 0).is_grain_aligned(G64BIT));
        assert!(!mk_ba(0x1000, 1).is_grain_aligned(G64BIT));
        assert!(!mk_ba(0x1001, 0).is_grain_aligned(G64BIT));
        assert!(!mk_ba(0x1001, 3).is_grain_aligned(G64BIT));

        assert!(mk_ba(0x1001, 0).is_grain_aligned(G8BIT));
        assert!(!mk_ba(0x1001, 3).is_grain_aligned(G8BIT));
    }

    #[test]
    fn test_is_normalized() {
        assert!(mk_ba(0x1000, 0).is_normalized(G64BIT));
        assert!(mk_ba(0x1000, 63).is_normalized(G64BIT));
        assert!(!mk_ba(0x1000, 64).is_normalized(G64BIT));
        assert!(!mk_ba(0x1001, 0).is_normalized(G64BIT));
        assert!(!mk_ba(0x1007, 0).is_normalized(G64BIT));
        assert!(!mk_ba(0x1007, 63).is_normalized(G64BIT));
        assert!(mk_ba(0x1008, 0).is_normalized(G64BIT));

        assert!(mk_ba(0x1000, 0).is_normalized(G8BIT));
        assert!(mk_ba(0x1000, 7).is_normalized(G8BIT));
        assert!(!mk_ba(0x1000, 8).is_normalized(G8BIT));
        assert!(mk_ba(0x1001, 0).is_normalized(G8BIT));
    }

    #[test]
    fn test_normalize() {
        assert_eq!(mk_ba(0x1000, 0).normalize(G64BIT), mk_ba(0x1000, 0));
        assert_eq!(mk_ba(0x1000, 63).normalize(G64BIT), mk_ba(0x1000, 63));
        assert_eq!(mk_ba(0x1000, 64).normalize(G64BIT), mk_ba(0x1008, 0));
        assert_eq!(mk_ba(0x1000, 65).normalize(G64BIT), mk_ba(0x1008, 1));
        assert_eq!(mk_ba(0x1001, 0).normalize(G64BIT), mk_ba(0x1000, 8));
        assert_eq!(mk_ba(0x1007, 0).normalize(G64BIT), mk_ba(0x1000, 56));
        assert_eq!(mk_ba(0x1007, 7).normalize(G64BIT), mk_ba(0x1000, 63));
        assert_eq!(mk_ba(0x1007, 8).normalize(G64BIT), mk_ba(0x1008, 0));
        assert_eq!(mk_ba(0x1007, 9).normalize(G64BIT), mk_ba(0x1008, 1));
    }

    #[test]
    fn test_empty_range() {
        for g in GS {
            let base = 0x1000;
            for bit in 0..g.bits() {
                let ba = mk_ba(base, bit);
                let result = break_range(g, ba, ba);
                assert!(
                    result.is_empty(),
                    "Not empty. bit: {bit}, result: {result:?}"
                );
            }
        }
    }

    #[test]
    fn test_subgrain_range() {
        for g in GS {
            let base = 0x1000;
            for bit0 in 0..g.bits() {
                let ba0 = mk_ba(base, bit0);
                for bit1 in (bit0 + 1)..g.bits() {
                    let ba1 = mk_ba(base, bit1);
                    let result = break_range(g, ba0, ba1);
                    assert_eq!(
                        result,
                        vec![VisitRange::SubGrain {
                            addr: mk_addr(base),
                            bit_start: bit0,
                            bit_end: bit1
                        }],
                        "Not equal.  bit0: {bit0}, bit1: {bit1}",
                    );
                }
            }
        }
    }

    #[test]
    fn test_end_grain_range() {
        for g in GS {
            let base = 0x1000;
            let ba1 = mk_ba(base + g.bytes(), 0);
            for bit0 in 1..g.bits() {
                let ba0 = mk_ba(base, bit0);
                let result = break_range(g, ba0, ba1);
                assert_eq!(
                    result,
                    vec![VisitRange::SubGrain {
                        addr: mk_addr(base),
                        bit_start: bit0,
                        bit_end: g.bits()
                    }],
                    "Not equal.  bit0: {bit0}",
                );
            }
        }
    }

    #[test]
    fn test_adjacent_grain_range() {
        for g in GS {
            let base = 0x1000;
            for bit0 in 1..g.bits() {
                let ba0 = mk_ba(base, bit0);
                for bit1 in 1..g.bits() {
                    let ba1 = mk_ba(base + g.bytes(), bit1);
                    let result = break_range(g, ba0, ba1);
                    assert_eq!(
                        result,
                        vec![
                            VisitRange::SubGrain {
                                addr: mk_addr(base),
                                bit_start: bit0,
                                bit_end: g.bits(),
                            },
                            VisitRange::SubGrain {
                                addr: mk_addr(base + g.bytes()),
                                bit_start: 0,
                                bit_end: bit1,
                            },
                        ],
                        "Not equal.  bit0: {bit0}, bit1: {bit1}",
                    );
                }
            }
        }
    }

    #[test]
    fn test_left_and_whole_range() {
        for g in GS {
            let base = 0x1000;
            for bit0 in 1..g.bits() {
                let ba0 = mk_ba(base, bit0);
                for word1 in 2..8 {
                    let ba1 = mk_ba(base + word1 * g.bytes(), 0);
                    let result = break_range(g, ba0, ba1);
                    assert_eq!(
                        result,
                        vec![
                            VisitRange::SubGrain {
                                addr: mk_addr(base),
                                bit_start: bit0,
                                bit_end: g.bits(),
                            },
                            VisitRange::WholeGrain {
                                start: mk_addr(base + g.bytes()),
                                end: mk_addr(base + word1 * g.bytes()),
                            },
                        ],
                        "Not equal.  bit0: {bit0}, word1: {word1}",
                    );
                }
            }
        }
    }

    #[test]
    fn test_whole_and_right_range() {
        for g in GS {
            let base = 0x1000;
            for word0 in 1..8 {
                let ba0 = mk_ba(base - word0 * g.bytes(), 0);
                for bit1 in 1..g.bits() {
                    let ba1 = mk_ba(base, bit1);
                    let result = break_range(g, ba0, ba1);
                    assert_eq!(
                        result,
                        vec![
                            VisitRange::WholeGrain {
                                start: mk_addr(base - word0 * g.bytes()),
                                end: mk_addr(base),
                            },
                            VisitRange::SubGrain {
                                addr: mk_addr(base),
                                bit_start: 0,
                                bit_end: bit1,
                            },
                        ],
                        "Not equal.  word0: {word0}, bit1: {bit1}",
                    );
                }
            }
        }
    }

    #[test]
    fn test_whole_range() {
        for g in GS {
            let base = 0x1000;
            let ba0 = mk_ba(base, 0);
            let ba1 = mk_ba(base + 42 * g.bytes(), 0);
            let result = break_range(g, ba0, ba1);
            assert_eq!(
                result,
                vec![VisitRange::WholeGrain {
                    start: mk_addr(base),
                    end: mk_addr(base + 42 * g.bytes()),
                },],
            );
        }
    }

    #[test]
    fn test_left_whole_right_range() {
        for g in GS {
            let base0 = 0x1000;
            let base1 = 0x2000;

            for bit0 in 1..g.bits() {
                let ba0 = mk_ba(base0 - g.bytes(), bit0);
                for bit1 in 1..g.bits() {
                    let ba1 = mk_ba(base1, bit1);
                    let result = break_range(g, ba0, ba1);
                    assert_eq!(
                        result,
                        vec![
                            VisitRange::SubGrain {
                                addr: mk_addr(base0 - g.bytes()),
                                bit_start: bit0,
                                bit_end: g.bits(),
                            },
                            VisitRange::WholeGrain {
                                start: mk_addr(base0),
                                end: mk_addr(base1),
                            },
                            VisitRange::SubGrain {
                                addr: mk_addr(base1),
                                bit_start: 0,
                                bit_end: bit1,
                            },
                        ],
                        "Not equal.  bit0: {bit0}, bit1: {bit1}",
                    );
                }
            }
        }
    }
}
