use super::freelist::*;
use super::memory::MmapStrategy;
use crate::util::address::Address;
use crate::util::constants::*;
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
    strategy: MmapStrategy,
}

impl FreeList for RawMemoryFreeList {
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
        debug_assert!(
            self.base + offset >= self.base && self.base + offset < self.high_water,
            "base={:?} offset={:?} index={:?} high_water={:?}",
            self.base,
            offset,
            self.base + offset,
            self.high_water
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
        self.units_per_block() - self.heads - 1
    }
    pub fn default_block_size(units: i32, heads: i32) -> i32 {
        usize::min(Self::size_in_pages(units, heads) as _, 16) as _
    }
    pub fn size_in_pages(units: i32, heads: i32) -> i32 {
        let map_size = ((units + heads + 1) as usize) << LOG_BYTES_IN_UNIT;
        conversions::bytes_to_pages_up(map_size as _) as _
    }

    pub fn new(
        base: Address,
        limit: Address,
        pages_per_block: i32,
        units: i32,
        grain: i32,
        heads: i32,
        strategy: MmapStrategy,
    ) -> Self {
        debug_assert!(units <= MAX_UNITS && heads <= MAX_HEADS);
        debug_assert!(
            base + conversions::pages_to_bytes(Self::size_in_pages(units, heads) as _) <= limit
        );
        Self {
            head: -1,
            heads,
            base,
            limit,
            high_water: base,
            max_units: units,
            grain,
            current_units: 0,
            pages_per_block,
            strategy,
        }
    }

    fn current_capacity(&self) -> i32 {
        let list_blocks = conversions::bytes_to_pages_up(self.high_water - self.base) as i32
            / self.pages_per_block;
        self.units_in_first_block() + (list_blocks - 1) * self.units_per_block()
    }

    pub fn grow_freelist(&mut self, units: i32) -> bool {
        let required_units = units + self.current_units;
        if required_units > self.max_units {
            return false;
        }
        let blocks = if required_units > self.current_capacity() {
            let units_requested = required_units - self.current_capacity();
            (units_requested + self.units_per_block() - 1) / self.units_per_block()
        } else {
            0
        };
        self.grow_list_by_blocks(blocks, required_units);
        true
    }
    fn grow_list_by_blocks(&mut self, blocks: i32, new_max: i32) {
        debug_assert!(
            (new_max <= self.grain) || (((new_max / self.grain) * self.grain) == new_max)
        );

        if blocks > 0 {
            // Allocate more VM from the OS
            self.raise_high_water(blocks);
        }

        let old_max = self.current_units;
        assert!(
            new_max <= self.current_capacity(),
            "blocks and new max are inconsistent: need more blocks for the requested capacity"
        );
        assert!(
            new_max <= self.max_units,
            "Requested list to grow larger than the configured maximum"
        );
        self.current_units = new_max;

        if old_max == 0 {
            // First allocation of capacity: initialize the sentinels.
            for i in 1..=self.heads {
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
        assert_ne!(
            self.high_water, self.limit,
            "Attempt to grow FreeList beyond limit"
        );
        if self.high_water + grow_extent > self.limit {
            grow_extent = self.high_water - self.limit;
        }
        self.mmap(self.high_water, grow_extent);
        self.high_water += grow_extent;
    }

    fn mmap(&self, start: Address, bytes: usize) {
        let res = super::memory::dzmmap_noreplace(start, bytes, self.strategy);
        assert!(res.is_ok(), "Can't get more space with mmap()");
    }
    pub fn get_limit(&self) -> Address {
        self.limit
    }
}

/**
 * See documentation of `mod tests` below for the necessity of `impl Drop`.
 */
#[cfg(test)]
impl Drop for RawMemoryFreeList {
    fn drop(&mut self) {
        let len = self.high_water - self.base;
        if len != 0 {
            unsafe {
                ::libc::munmap(self.base.as_usize() as _, len);
            }
        }
    }
}

/**
 * The initialization of `RawMemoryFreeList` involves memory-mapping a fixed range of virtual address.
 *
 * This raises an implicit assumption that a test process can only have one `RawMemoryFreeList` instance at a time unless each instance uses different fixed address ranges.
 *
 * We use a single fixed address range for all the following tests. So the tests cannot be executed in parallel. Which means:
 *
 * 1. Each test should hold a global mutex to prevent parallel execution.
 * 2. `RawMemoryFreeList` should implement `Drop` trait to unmap the memory properly at the end of each test.
 */
#[cfg(test)]
mod tests {
    use super::FreeList;
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    const TOP_SENTINEL: i32 = -1;
    const FIRST_UNIT: i32 = 0;

    lazy_static! {
        /**
         * See documentation of `mod tests` above for for the necessity of this mutex.
         */
        static ref MUTEX: Mutex<()> = Mutex::new(());
    }

    fn new_raw_memory_freelist<'a>(
        list_size: usize,
        grain: i32,
    ) -> (MutexGuard<'a, ()>, RawMemoryFreeList, i32, i32, i32) {
        /*
         * Note: The mutex could be poisoned!
         * Test `free_list_access_out_of_bounds` below is expected to panic and poison the mutex.
         * So we need to manually recover the lock here, if it is poisoned.
         *
         * See documentation of `mod tests` above for more details.
         */
        let guard = match MUTEX.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let start = crate::util::test_util::RAW_MEMORY_FREELIST_TEST_REGION.start;
        let extent = BYTES_IN_PAGE;
        let pages_per_block = RawMemoryFreeList::default_block_size(list_size as _, 1);
        assert_eq!(pages_per_block, 1);
        let mut l = RawMemoryFreeList::new(
            start,
            start + extent,
            pages_per_block,
            list_size as _,
            grain,
            1,
            MmapStrategy::Normal,
        );
        // Grow the free-list to do the actual memory-mapping.
        l.grow_freelist(list_size as _);
        let last_unit = list_size as i32 - grain;
        let bottom_sentinel = list_size as i32;
        (guard, l, list_size as _, last_unit, bottom_sentinel)
    }

    #[test]
    #[allow(clippy::cognitive_complexity)] // extensive checks, and it doesn't matter for tests
    fn new_free_list_grain1() {
        let (_guard, l, _, last_unit, bottom_sentinel) = new_raw_memory_freelist(5, 1);
        assert_eq!(l.head(), TOP_SENTINEL);

        assert_eq!(l.get_prev(TOP_SENTINEL), last_unit);
        assert_eq!(l.get_next(TOP_SENTINEL), FIRST_UNIT);

        assert_eq!(l.get_size(FIRST_UNIT), 1);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), -1);
        assert_eq!(l.get_right(FIRST_UNIT), 1);
        assert_eq!(l.get_next(FIRST_UNIT), 1);
        assert!(l.is_free(FIRST_UNIT));
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(!l.is_multi(FIRST_UNIT));

        assert_eq!(l.get_size(1), 1);
        assert_eq!(l.get_left(1), 0);
        assert_eq!(l.get_prev(1), 0);
        assert_eq!(l.get_right(1), 2);
        assert_eq!(l.get_next(1), 2);
        assert!(l.is_free(1));
        assert!(l.is_coalescable(1));
        assert!(!l.is_multi(1));

        assert_eq!(l.get_size(last_unit), 1);
        assert_eq!(l.get_left(last_unit), last_unit - 1);
        assert_eq!(l.get_prev(last_unit), last_unit - 1);
        assert_eq!(l.get_right(last_unit), bottom_sentinel);
        assert_eq!(l.get_next(last_unit), -1);
        assert!(l.is_free(last_unit));
        assert!(l.is_coalescable(last_unit));
        assert!(!l.is_multi(last_unit));

        assert_eq!(l.get_prev(bottom_sentinel), bottom_sentinel);
        assert_eq!(l.get_next(bottom_sentinel), bottom_sentinel);
    }

    #[test]
    #[allow(clippy::cognitive_complexity)] // extensive checks, and it doesn't matter for tests
    fn new_free_list_grain2() {
        let (_guard, l, _, last_unit, bottom_sentinel) = new_raw_memory_freelist(6, 2);
        assert_eq!(l.head(), TOP_SENTINEL);

        assert_eq!(l.get_prev(TOP_SENTINEL), last_unit);
        assert_eq!(l.get_next(TOP_SENTINEL), FIRST_UNIT);

        assert_eq!(l.get_size(FIRST_UNIT), 2);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), -1);
        assert_eq!(l.get_right(FIRST_UNIT), 2);
        assert_eq!(l.get_next(FIRST_UNIT), 2);
        assert!(l.is_free(FIRST_UNIT));
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(l.is_multi(FIRST_UNIT));

        assert_eq!(l.get_size(2), 2);
        assert_eq!(l.get_left(2), 0);
        assert_eq!(l.get_prev(2), 0);
        assert_eq!(l.get_right(2), 4);
        assert_eq!(l.get_next(2), 4);
        assert!(l.is_free(2));
        assert!(l.is_coalescable(2));
        assert!(l.is_multi(2));

        assert_eq!(l.get_size(last_unit), 2);
        assert_eq!(l.get_left(last_unit), last_unit - 2);
        assert_eq!(l.get_prev(last_unit), last_unit - 2);
        assert_eq!(l.get_right(last_unit), bottom_sentinel);
        assert_eq!(l.get_next(last_unit), -1);
        assert!(l.is_free(last_unit));
        assert!(l.is_coalescable(last_unit));
        assert!(l.is_multi(last_unit));

        assert_eq!(l.get_prev(bottom_sentinel), bottom_sentinel);
        assert_eq!(l.get_next(bottom_sentinel), bottom_sentinel);
    }

    #[test]
    #[should_panic]
    fn free_list_access_out_of_bounds() {
        let (_guard, l, _, _, _) = new_raw_memory_freelist(5, 1);
        l.get_size(4096);
        // `_guard` should be dropped during stack unwinding
    }

    #[test]
    fn alloc_fit() {
        let (_guard, mut l, _, last_unit, _) = new_raw_memory_freelist(6, 2);
        let result = l.alloc(2);
        assert_eq!(result, 0);

        const NEXT: i32 = 2;

        assert_eq!(l.get_prev(TOP_SENTINEL), last_unit);
        assert_eq!(l.get_next(TOP_SENTINEL), NEXT);

        assert_eq!(l.get_size(FIRST_UNIT), 2);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), -1);
        assert_eq!(l.get_right(FIRST_UNIT), 2);
        assert_eq!(l.get_next(FIRST_UNIT), 2);
        assert!(!l.is_free(FIRST_UNIT)); // not free
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(l.is_multi(FIRST_UNIT));

        assert_eq!(l.get_size(2), 2);
        assert_eq!(l.get_left(2), 0);
        assert_eq!(l.get_prev(2), -1); // no prev now
        assert_eq!(l.get_right(2), 4);
        assert_eq!(l.get_next(2), 4);
        assert!(l.is_free(2));
        assert!(l.is_coalescable(2));
        assert!(l.is_multi(2));
    }

    #[test]
    #[allow(clippy::cognitive_complexity)] // extensive checks, and it doesn't matter for tests
    fn alloc_split() {
        let (_guard, mut l, _, last_unit, _) = new_raw_memory_freelist(6, 2);
        let result = l.alloc(1);
        assert_eq!(result, 0);

        const NEXT: i32 = 1;
        assert_eq!(l.get_prev(TOP_SENTINEL), last_unit);
        assert_eq!(l.get_next(TOP_SENTINEL), NEXT);

        assert_eq!(l.get_size(FIRST_UNIT), 1);
        assert_eq!(l.get_left(FIRST_UNIT), -1);
        assert_eq!(l.get_prev(FIRST_UNIT), 1); // prev is 1 now
        assert_eq!(l.get_right(FIRST_UNIT), 1); // right is 1 now
        assert_eq!(l.get_next(FIRST_UNIT), 2);
        assert!(!l.is_free(FIRST_UNIT)); // not free
        assert!(l.is_coalescable(FIRST_UNIT));
        assert!(!l.is_multi(FIRST_UNIT)); // not multi

        assert_eq!(l.get_size(1), 1);
        assert_eq!(l.get_left(1), 0); // unit1's left is 0
        assert_eq!(l.get_prev(1), -1); // unit1's prev is -1 (no prev, unit1 is removed form the list)
        assert_eq!(l.get_right(1), 2);
        assert_eq!(l.get_next(1), 2);
        assert!(l.is_free(1)); // not free
        assert!(l.is_coalescable(1));
        assert!(!l.is_multi(1)); // not multi

        assert_eq!(l.get_size(2), 2);
        assert_eq!(l.get_left(2), 1);
        assert_eq!(l.get_prev(2), 1); // uni2's prev is 1 now
        assert_eq!(l.get_right(2), 4);
        assert_eq!(l.get_next(2), 4);
        assert!(l.is_free(2));
        assert!(l.is_coalescable(2));
        assert!(l.is_multi(2));
    }

    #[test]
    fn alloc_split_twice() {
        let (_guard, mut l, _, _, _) = new_raw_memory_freelist(6, 2);
        // Alloc size 1 and cause split
        let res1 = l.alloc(1);
        assert_eq!(res1, 0);
        // Alloc size 1
        let res2 = l.alloc(1);
        assert_eq!(res2, 1);

        // Next available unit has no prev now
        assert_eq!(l.get_prev(2), -1);
    }

    #[test]
    fn alloc_skip() {
        let (_guard, mut l, _, _, _) = new_raw_memory_freelist(6, 2);
        // Alloc size 1 and cause split
        let res1 = l.alloc(1);
        assert_eq!(res1, 0);
        // Alloc size 2, we skip unit1
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // unit1 is still free, and linked with unit4
        assert!(l.is_free(1));
        assert_eq!(l.get_next(1), 4);
        assert_eq!(l.get_prev(4), 1);
    }

    #[test]
    fn alloc_exhaust() {
        let (_guard, mut l, _, _, _) = new_raw_memory_freelist(6, 2);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);
        let res3 = l.alloc(2);
        assert_eq!(res3, 4);
        let res4 = l.alloc(2);
        assert_eq!(res4, FAILURE);
    }

    #[test]
    fn free_unit() {
        let (_guard, mut l, _, _, _) = new_raw_memory_freelist(6, 2);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // Unit4 is still free, but has no prev
        assert_eq!(l.get_prev(4), -1);

        // Free Unit2
        let freed = l.free(res2, false);
        assert_eq!(freed, res2);
        assert!(l.is_free(res2));
    }

    #[test]
    fn free_coalesce() {
        let (_guard, mut l, _, _, _) = new_raw_memory_freelist(6, 2);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // Free Unit2. It will coalesce with Unit4
        let coalesced_size = l.free(res2, true);
        assert_eq!(coalesced_size, 4);
    }

    #[test]
    fn free_cant_coalesce() {
        let (_guard, mut l, _, _, _) = new_raw_memory_freelist(6, 2);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);
        let res3 = l.alloc(1);
        assert_eq!(res3, 4);

        // Free Unit2. It cannot coalesce with Unit4
        let coalesced_size = l.free(res2, true);
        assert_eq!(coalesced_size, 2);
    }

    #[test]
    fn free_realloc() {
        let (_guard, mut l, _, _, _) = new_raw_memory_freelist(6, 2);
        let res1 = l.alloc(2);
        assert_eq!(res1, 0);
        let res2 = l.alloc(2);
        assert_eq!(res2, 2);

        // Unit4 is still free, but has no prev
        assert_eq!(l.get_prev(4), -1);

        // Free Unit2
        let freed = l.free(res2, false);
        assert_eq!(freed, res2);
        assert!(l.is_free(res2));

        // Alloc again
        let res3 = l.alloc(2);
        assert_eq!(res3, 2);
        assert!(!l.is_free(res3));

        let res4 = l.alloc(1);
        assert_eq!(res4, 4);
    }
}
