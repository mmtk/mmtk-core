use super::generic_freelist::*;
use crate::util::constants::*;
use crate::util::address::Address;
use crate::util::conversions;


/** log2 of the number of bits used by a free list entry (two entries per unit) */
const LOG_ENTRY_BITS: usize = LOG_BITS_IN_INT as _;

/** log2 of the number of bytes used by a free list entry (two entries per unit) */
const LOG_BYTES_IN_ENTRY: usize = LOG_ENTRY_BITS - (LOG_BITS_IN_BYTE as usize);

/** log2 of the number of bytes used by a free list unit */
const LOG_BYTES_IN_UNIT: usize = LOG_BYTES_IN_ENTRY + 1;


#[derive(Debug)]
pub struct RawMemoryFreeList {
    pub head: i32,
    pub heads: i32,
    base: Address,
    limit: Address,
    high_water: Address,
    max_units: i32,
    grain: i32,
    current_units: i32,
    pages_per_block: i32,
}

impl GenericFreeList for RawMemoryFreeList {
    fn head(&self) -> i32 {
        self.head
    }
    fn heads(&self) -> i32 {
        self.heads
    }
    fn get_entry(&self, index: i32) -> i32 {
        let offset = (index << LOG_BYTES_IN_ENTRY) as usize;
        debug_assert!(self.base + offset >= self.base && self.base + offset < self.high_water);
        unsafe { (self.base + offset).load() }
    }
    fn set_entry(&mut self, index: i32, value: i32) {
        let offset = (index << LOG_BYTES_IN_ENTRY) as usize;
        debug_assert!(self.base + offset >= self.base && self.base + offset < self.high_water,
            "base={:?} offset={:?} index={:?} high_water={:?}",
            self.base, offset, self.base + offset, self.high_water
        );
        unsafe { (self.base + offset).store(value) }
    }
    fn alloc(&mut self, size: i32) -> i32 {
        if self.current_units == 0 {
            return FAILURE;
        }
        let mut unit = self.head();
        let mut s = 0;
        while ({
            unit = self.get_next(unit);
            unit != self.head()
        }) && ({
            s = self.get_size(unit);
            s < size
        }) {}
        if unit == self.head() {
            FAILURE
        } else {
            self.__alloc(size, unit, s)
        }
    }
}

impl RawMemoryFreeList {
    fn units_per_block(&self) -> i32 {
        (conversions::pages_to_bytes(self.pages_per_block as _) >> LOG_BYTES_IN_UNIT) as _
    }
    fn units_in_first_block(&self) -> i32 {
        self.units_per_block() - (self.heads as i32) - 1
    }
    pub fn default_block_size(units: i32, heads: i32) -> i32 {
        return usize::min(Self::size_in_pages(units, heads) as _, 16) as _;
    }
    pub fn size_in_pages(units: i32, heads: i32) -> i32 {
        let map_size = ((units + heads + 1) as usize) << LOG_BYTES_IN_UNIT as usize;
        conversions::bytes_to_pages_up(map_size as _) as _
    }
    
    pub fn new(base: Address, limit: Address, pages_per_block: i32, units: i32, grain: i32, heads: i32) -> Self {
        debug_assert!(units <= MAX_UNITS && heads <= MAX_HEADS);
        debug_assert!(base + conversions::pages_to_bytes(Self::size_in_pages(units,heads) as _) <= limit);
        Self {
            head: -1,
            heads: heads,
            base: base,
            limit: limit,
            high_water: base,
            max_units: units,
            grain: grain,
            current_units: 0,
            pages_per_block: pages_per_block,
        }
    }

    fn current_capacity(&self) -> i32 {
        let list_blocks = conversions::bytes_to_pages(self.high_water - self.base) as i32 / self.pages_per_block;
        return self.units_in_first_block() + (list_blocks - 1) * self.units_per_block();
    }
    
    pub fn grow_freelist(&mut self, units: i32) -> bool {
        let required_units = units + self.current_units;
        if required_units > self.max_units {
          return false;
        }
        let mut blocks = 0;
        if required_units > self.current_capacity() {
          let units_requested = required_units - self.current_capacity();
          blocks = (units_requested + self.units_per_block() - 1) / self.units_per_block();
        }
        self.grow_list_by_blocks(blocks, required_units);
        return true;
    }
    fn grow_list_by_blocks(&mut self, blocks: i32, new_max: i32) {
        debug_assert!((new_max <= self.grain) || (((new_max / self.grain) * self.grain) == new_max));
    
        if blocks > 0 {
            // Allocate more VM from the OS
            self.raise_high_water(blocks);
        }
    
        let old_max = self.current_units;
        assert!(new_max <= self.current_capacity(),
            "blocks and new max are inconsistent: need more blocks for the requested capacity");
        assert!(new_max <= self.max_units,
            "Requested list to grow larger than the configured maximum");
        self.current_units = new_max;
    
        if old_max == 0 {
          // First allocation of capacity: initialize the sentinels.
          for i in 1..(self.heads+1) {
            self.set_sentinel(-i);
          }
        } else {
          // Turn the old top-of-heap sentinel into a single used block
          self.set_size(old_max, 1);
        }
    
        if new_max == 0 {
          return;
        }
    
        // Set a sentinel at the top of the new range
        self.set_sentinel(new_max);
    
        let mut cursor = new_max;
    
        /* A series of grain size regions in the middle */
        let grain = i32::min(self.grain, new_max - old_max);
        cursor -= grain;
        while cursor >= old_max {
            self.set_size(cursor, grain);
            self.add_to_free(cursor);
            cursor -= grain;
        }
    }

    fn raise_high_water(&mut self, blocks: i32) {
        let mut grow_extent = conversions::pages_to_bytes((self.pages_per_block * blocks) as _);
        assert_ne!(self.high_water, self.limit, "Attempt to grow FreeList beyond limit");
        if self.high_water + grow_extent > self.limit {
            grow_extent = self.high_water - self.limit;
        }
        self.mmap(self.high_water, grow_extent);
        self.high_water = self.high_water + grow_extent;
    }

    fn mmap(&self, start: Address, bytes: usize) {
        if let Err(_) = super::memory::dzmmap(start, bytes) {
            assert!(false, "Can't get more space with mmap()");
        }
    }
    pub fn get_limit(&self) -> Address {
        return self.limit;
    }
}
