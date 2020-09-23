use crate::util::Address;
use crate::util::constants::*;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::*;
use std::ops::Deref;
use std::sync::atomic::{AtomicU8, Ordering};



pub trait PerChunkMetadata: Sized + 'static {
    const LOG_BYTES_IN_CHUNK: usize = LOG_BYTES_IN_CHUNK;
    const BYTES_IN_CHUNK: usize = 1 << Self::LOG_BYTES_IN_CHUNK;

    const METADATA_PAGES_PER_CHUNK: usize;

    fn of(address: Address) -> &'static Self {
        unsafe { &*conversions::chunk_align_down(address).to_ptr::<Self>() }
    }
}

#[repr(C)]
pub struct MarkBitMap([AtomicU8; BYTES_IN_BITMAP]);

impl MarkBitMap {
    fn calculate_bit_location(a: Address) -> (usize, usize) {
        let chunk_start = conversions::chunk_align_down(a);
        debug_assert!(chunk_start <= a);
        debug_assert!(a <= chunk_start + BYTES_IN_CHUNK);
        let offset_in_words = (a - chunk_start) >> LOG_BYTES_IN_WORD;
        let byte_index = offset_in_words >> LOG_BITS_IN_BYTE;
        let bit_index = offset_in_words & (BITS_IN_BYTE - 1);
        (byte_index, bit_index)
    }
    pub fn attempt_mark(&self, a: Address) -> bool {
        let (byte_index, bit_index) = Self::calculate_bit_location(a);
        let slot: &AtomicU8 = &self[byte_index];
        let old_value = slot.fetch_or(1 << bit_index, Ordering::SeqCst);
        old_value & (1 << bit_index) == 0
    }
    pub fn clear(&self) {
        for slot in self.iter() {
            slot.store(0, Ordering::SeqCst);
        }
    }
}

const BITS_IN_BITMAP: usize = <MarkBitMap as PerChunkMetadata>::BYTES_IN_CHUNK >> LOG_BYTES_IN_WORD;
const BYTES_IN_BITMAP: usize = conversions::raw_align_up(BITS_IN_BITMAP, BITS_IN_BYTE) >> LOG_BITS_IN_BYTE;
const PAGES_IN_BITMAP: usize = (BYTES_IN_BITMAP + BYTES_IN_PAGE - 1) >> LOG_BYTES_IN_PAGE;

impl PerChunkMetadata for MarkBitMap {
    const METADATA_PAGES_PER_CHUNK: usize = PAGES_IN_BITMAP;
}

impl Deref for MarkBitMap {
    type Target = [AtomicU8; BYTES_IN_BITMAP];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialEq for MarkBitMap {
    fn eq(&self, other: &Self) -> bool {
        self as *const _ == other as * const _
    }
}