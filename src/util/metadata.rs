use super::constants::*;
use super::conversions;
use super::heap::layout::vm_layout_constants::*;
use super::{memory::dzmmap, Address};
use crate::SelectedConstraints;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const METADATA_BASE: Address = HEAP_END;

pub struct BitsReference {
    base: Address,
    word_offset: usize,
    bit_offset: usize,
    mask: usize,
}

impl BitsReference {
    /// `log_granularity`: Logarithmic value of the bytes granularity.
    ///
    /// `log_bits`: Logarithmic value of number of bits this struct is referencing
    ///
    /// **Invariant:** `log_granularity` + 3 >= `log_bits`
    ///
    /// _e.g. for {8-byte -> 1-bit} mapping, `log_granularity` = 3 (i.e. log2(8 bytes)), `log_bits` = 0  (i.e. log2(1 bit))._
    pub const fn of(addr: Address, log_granularity: u8, log_bits: u8) -> Self {
        let base = metadata_start(addr);
        let unit_index = (addr.as_usize() & (BYTES_IN_CHUNK - 1)) >> log_granularity;
        let log_units_in_metadata_word = LOG_BITS_IN_WORD - (log_bits as usize);
        let word_offset = unit_index >> log_units_in_metadata_word << LOG_BYTES_IN_WORD;
        let bit_offset = (unit_index & ((1 << log_units_in_metadata_word) - 1)) << log_bits;

        Self {
            base,
            word_offset,
            bit_offset,
            mask: ((1 << (1 << log_bits)) - 1) << bit_offset,
        }
    }

    /// Return `true` if the CAS operation is successful. Otherwise return `false`.
    #[inline(always)]
    pub fn attempt(&self, old: usize, new: usize) -> bool {
        let old = old << self.bit_offset;
        let new = new << self.bit_offset;
        debug_assert!((old & !self.mask) == 0);
        debug_assert!((new & !self.mask) == 0);
        let word = unsafe { &*((self.base.as_usize() + self.word_offset) as *const AtomicUsize) };
        loop {
            let old_word = word.load(Ordering::SeqCst);
            if (old_word & self.mask) != old {
                return false;
            }
            let new_word = (old_word & !self.mask) | new;
            if old_word == word.compare_and_swap(old_word, new_word, Ordering::SeqCst) {
                return true;
            }
        }
    }
}

const fn metadata_start(address: Address) -> Address {
    let chunk_index = conversions::address_to_chunk_index(address);
    let offset = (chunk_index * SelectedConstraints::METADATA_PAGES_PER_CHUNK) << LOG_BYTES_IN_PAGE;
    unsafe { Address::from_usize(METADATA_BASE.as_usize() + offset) }
}

pub fn map_metadata_pages_for_chunk(chunk: Address) {
    let metadata_start = metadata_start(chunk);
    dzmmap(
        metadata_start,
        SelectedConstraints::METADATA_PAGES_PER_CHUNK << LOG_BYTES_IN_PAGE,
    )
    .unwrap();
}
