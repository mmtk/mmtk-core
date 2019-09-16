use ::util::Address;
use super::allocator::{align_allocation_no_fill, fill_alignment_gap};
use ::util::alloc::Allocator;
use ::util::heap::FreeListPageResource;
use policy::region::*;
use libc::c_void;

type PR = FreeListPageResource<RegionSpace>;

const USE_TLABS: bool = true;
const MIN_TLAB_SIZE: usize = 2 * 1024;
const MAX_TLAB_SIZE: usize = ::plan::SelectedConstraints::MAX_NON_LOS_COPY_BYTES;

#[repr(C)]
#[derive(Debug)]
pub struct RegionAllocator {
    pub tls: *mut c_void,
    cursor: Address,
    limit: Address,
    pub space: &'static mut RegionSpace,
    refills: usize,
    tlab_size: usize,
    generation: Gen,
}

impl RegionAllocator {
    pub fn adjust_tlab_size(&mut self) {
        if USE_TLABS {
            let factor = self.refills as f32 / 50f32;
            self.tlab_size = (self.tlab_size as f32 * factor) as usize;
            if self.tlab_size < MIN_TLAB_SIZE {
                self.tlab_size = MIN_TLAB_SIZE;
            } else if self.tlab_size > MAX_TLAB_SIZE {
                self.tlab_size = MAX_TLAB_SIZE;
            }
            self.refills = 0;
        }
    }

    pub fn reset(&mut self) {
        self.retire_tlab();
        self.cursor = unsafe { Address::zero() };
        self.limit = unsafe { Address::zero() };
    }
}

impl Allocator<PR> for RegionAllocator {
    fn get_space(&self) -> Option<&'static RegionSpace> {
        Some(unsafe { &*(self.space as *const _) })
    }

    #[inline]
    fn alloc(&mut self, bytes: usize, align: usize, offset: isize) -> Address {
        debug_assert!(bytes <= BYTES_IN_REGION);
        trace!("alloc");
        let start = align_allocation_no_fill(self.cursor, align, offset);
        let end = start + bytes;
        // check whether we've exceeded the limit
        if end > self.limit {
            return self.alloc_slow(bytes, align, offset);
        }
        // sufficient memory is available, so we can finish performing the allocation
        fill_alignment_gap(self.cursor, start);
        self.cursor = end;
        // Region::of(start).cursor = end;
        start
    }

    fn alloc_slow_once(&mut self, bytes: usize, align: usize, offset: isize) -> Address {
        trace!("alloc_slow");
        debug_assert!(bytes <= BYTES_IN_REGION);
        if USE_TLABS {
            let mut size = if bytes > self.tlab_size { bytes } else { self.tlab_size };
            let mut tlabs = size / MIN_TLAB_SIZE;
            if tlabs * MIN_TLAB_SIZE < size {
                tlabs += 1;
            }
            size = tlabs * MIN_TLAB_SIZE;
            debug_assert!(size >= bytes);
            match self.space.refill(self.tls, size, self.generation) {
                Some(tlab) => {
                    self.refills += 1;
                    self.retire_tlab();
                    self.cursor = tlab;
                    self.limit = self.cursor + size;
                    self.init_offsets(self.cursor, self.limit);
                    self.alloc(bytes, align, offset)
                },
                None => unsafe { Address::zero() },
            }
        } else {
            match self.space.acquire_new_region(self.tls, self.generation) {
                Some(region) => {
                    self.cursor = region.start();
                    self.limit = self.cursor + BYTES_IN_REGION;
                    self.alloc(bytes, align, offset)
                },
                None => unsafe { Address::zero() },
            }

        }
    }

    fn get_tls(&self) -> *mut c_void {
        self.tls
    }
}

impl RegionAllocator {
    pub fn new(tls: *mut c_void, space: &'static mut RegionSpace, generation: Gen) -> Self {
        RegionAllocator {
            tls,
            cursor: unsafe { Address::zero() },
            limit: unsafe { Address::zero() },
            space,
            tlab_size: (MIN_TLAB_SIZE + MAX_TLAB_SIZE) / 2,
            refills: 0,
            generation,
        }
    }

    fn init_offsets(&self, start: Address, limit: Address) {
        let mut region = Region::of(start);
        let region_start = region.start();
        debug_assert!(limit <= region_start + BYTES_IN_REGION);
        let mut cursor = start;
        while cursor < limit {
            debug_assert!(cursor >= region_start);
            let index = (cursor - region_start) >> LOG_BYTES_IN_CARD;
            region.card_offset_table[index] = start;
            cursor += BYTES_IN_CARD;
        }
    }

    fn retire_tlab(&self) {
        if USE_TLABS {
            let (cursor, end) = (self.cursor, self.limit);
            if cursor.is_zero() || end.is_zero() {
                return;
            }
            fill_alignment_gap(cursor, end);
        }
    }
}
