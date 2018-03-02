use ::util::address::Address;

use ::policy::space::Space;

use ::util::constants::*;

// FIXME: Put this somewhere more appropriate
pub const ALIGNMENT_VALUE: usize = 0xdeadbeef;
pub const LOG_MIN_ALIGNMENT: usize = LOG_BYTES_IN_INT as usize;
pub const MIN_ALIGNMENT: usize = 1 << LOG_MIN_ALIGNMENT;
#[cfg(target_arch = "x86")]
pub const LOG_MAX_ALIGNMENT: usize = 1 + LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;
#[cfg(target_arch = "x86_64")]
pub const LOG_MAX_ALIGNMENT: usize = LOG_BYTES_IN_LONG as usize - LOG_BYTES_IN_INT as usize;
pub const MAX_ALIGNMENT: usize = 1 << LOG_MAX_ALIGNMENT;

#[inline(always)]
pub fn align_allocation(region: Address, align: usize, offset: isize) -> Address {
    let region_isize = region.as_usize() as isize;

    let mask = (align - 1) as isize; // fromIntSignExtend
    let neg_off = -offset; // fromIntSignExtend
    let delta = (neg_off - region_isize) & mask;

    region + delta
}

#[inline(always)]
pub fn fill_alignment_gap(immut_start: Address, end: Address) {
    let mut start = immut_start;

    if MAX_ALIGNMENT - MIN_ALIGNMENT == BYTES_IN_INT {
        // At most a single hole
        if end - start != 0 {
            unsafe {
                start.store(ALIGNMENT_VALUE);
            }
        }
    } else {
        while start < end {
            unsafe {
                start.store(ALIGNMENT_VALUE);
            }
            start += BYTES_IN_INT;
        }
    }
}

#[inline(always)]
pub fn get_maximum_aligned_size(size: usize, alignment: usize, known_alignment: usize) -> usize {
    trace!("size={}, alignment={}, known_alignment={}, MIN_ALIGNMENT={}", size, alignment,
           known_alignment, MIN_ALIGNMENT);
    debug_assert!(size == size & !(known_alignment - 1));
    debug_assert!(known_alignment >= MIN_ALIGNMENT);

    if MAX_ALIGNMENT <= MIN_ALIGNMENT || alignment <= known_alignment {
        return size;
    } else {
        return size + alignment - known_alignment;
    }
}

pub trait Allocator<'a, T> where T: Space {
    fn get_space(&self) -> Option<&'a T>;

    fn alloc(&mut self, size: usize, align: usize, offset: isize) -> Address;

    fn alloc_slow(&mut self, size: usize, align: usize, offset: isize) -> Address;
}