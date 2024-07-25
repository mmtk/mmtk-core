//! Mechanisms for breaking a range in a bitmap into whole-grain and sub-grain ranges to maximize
//! bulk bitmap access.
//!
//! In this module, the term "grain" refers to the unit at which we access a bitmap. It can be a
//! byte (u8), a word (usize) or other power-of-two byte sizes.
//!
//! The `std::simd` module is still a nightly feature as of Rust 1.79.  When it stablizes, we can
//! allow the granularity to be a SIMD vector, too.

use num_traits::Num;

use crate::util::{constants::BITS_IN_BYTE, Address};

/// Offset of bits in a grain.
type BitOffset = usize;

/// Bit address within a grain.
#[derive(Debug, Clone, Copy)]
pub struct BitAddress {
    pub addr: Address,
    pub bit: BitOffset,
}

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
            log_bytes: log_bits >> BITS_IN_BYTE,
        }
    }

    pub fn of_type<T: Num>() -> Self {
        let bytes = std::mem::size_of::<T>();
        let log_bytes = bytes.ilog2() as usize;

        debug_assert_eq!(1 << log_bytes, bytes, "Not power-of-two size: {bytes}");

        Self::of_log_bytes(log_bytes)
    }

    pub const fn log_bytes(&self) -> usize {
        self.log_bytes
    }

    pub const fn log_bits(&self) -> usize {
        self.log_bytes() << BITS_IN_BYTE
    }

    pub const fn bytes(&self) -> usize {
        1 << self.log_bytes()
    }

    pub const fn bits(&self) -> usize {
        1 << self.log_bits()
    }
}

impl BitAddress {
    pub fn is_grain_aligned(&self, granularity: Granularity) -> bool {
        self.bit == 0 && self.addr.is_aligned_to(granularity.bytes())
    }

    pub fn is_normalized(&self, granularity: Granularity) -> bool {
        self.addr.is_aligned_to(granularity.bytes()) && self.bit < granularity.bytes()
    }

    pub fn normalize(&self, granularity: Granularity) -> Self {
        let addr = self.addr.align_down(granularity.bytes());
        let rem_bytes = addr.as_usize() % granularity.bytes();
        let bit = self.bit + rem_bytes * BITS_IN_BYTE;
        Self { addr, bit }
    }

    pub fn align_down_to_grain(&self, granularity: Granularity) -> Address {
        debug_assert!(self.is_normalized(granularity));
        self.addr.align_down(granularity.bytes())
    }

    pub fn align_up_to_grain(&self, granularity: Granularity) -> Address {
        debug_assert!(self.is_normalized(granularity));
        if self.bit == 0 {
            (self.addr + 1usize).align_up(granularity.bytes())
        } else {
            self.addr.align_up(granularity.bytes())
        }
    }
}

#[derive(Debug, Clone, Copy)]
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

    if start.addr == end.addr {
        // The start and the end are in the same grain.  Yield only one SubGrain range.
        return vec![VisitRange::SubGrain {
            addr: start.addr,
            bit_start: start.bit,
            bit_end: end.bit,
        }];
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
