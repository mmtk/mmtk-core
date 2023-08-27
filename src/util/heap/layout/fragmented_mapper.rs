use super::mmapper::MapState;
use super::Mmapper;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout::*;
use crate::util::memory::MmapStrategy;
use crate::util::Address;
use atomic::{Atomic, Ordering};
use std::cell::UnsafeCell;
use std::fmt;
use std::io::Result;
use std::mem::transmute;
use std::sync::Mutex;

const MMAP_NUM_CHUNKS: usize = 1 << (33 - LOG_MMAP_CHUNK_BYTES);

// 36 = 128G - physical memory larger than this is uncommon
// 40 = 2T. Increased to 2T. Though we probably won't use this much memory, we allow quarantine memory range,
// and that is usually used to quarantine a large amount of memory.
const LOG_MAPPABLE_BYTES: usize = 40;

/*
 * Size of a slab.  The value 10 gives a slab size of 1GB, with 1024
 * chunks per slab, ie a 1k slab map.  In a 64-bit address space, this
 * will require 1M of slab maps.
 */
const LOG_MMAP_CHUNKS_PER_SLAB: usize = 8;
const LOG_MMAP_SLAB_BYTES: usize = LOG_MMAP_CHUNKS_PER_SLAB + LOG_MMAP_CHUNK_BYTES;
const MMAP_SLAB_EXTENT: usize = 1 << LOG_MMAP_SLAB_BYTES;
const MMAP_SLAB_MASK: usize = (1 << LOG_MMAP_SLAB_BYTES) - 1;
/**
 * Maximum number of slabs, which determines the maximum mappable address space.
 */
const LOG_MAX_SLABS: usize = LOG_MAPPABLE_BYTES - LOG_MMAP_CHUNK_BYTES - LOG_MMAP_CHUNKS_PER_SLAB;
const MAX_SLABS: usize = 1 << LOG_MAX_SLABS;
/**
 * Parameters for the slab table.  The hash function requires it to be
 * a power of 2.  Must be larger than MAX_SLABS for hashing to work,
 * and should be much larger for it to be efficient.
 */
const LOG_SLAB_TABLE_SIZE: usize = 1 + LOG_MAX_SLABS;
const HASH_MASK: usize = (1 << LOG_SLAB_TABLE_SIZE) - 1;
const SLAB_TABLE_SIZE: usize = 1 << LOG_SLAB_TABLE_SIZE;
const SENTINEL: Address = Address::MAX;

type Slab = [Atomic<MapState>; MMAP_NUM_CHUNKS];

pub struct FragmentedMapper {
    lock: Mutex<()>,
    inner: UnsafeCell<InnerFragmentedMapper>,
}

unsafe impl Send for FragmentedMapper {}
unsafe impl Sync for FragmentedMapper {}

struct InnerFragmentedMapper {
    free_slab_index: usize,
    free_slabs: Vec<Option<Box<Slab>>>,
    slab_table: Vec<Option<Box<Slab>>>,
    slab_map: Vec<Address>,
    strategy: Atomic<MmapStrategy>,
}

impl fmt::Debug for FragmentedMapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FragmentedMapper({})", MMAP_NUM_CHUNKS)
    }
}

impl Mmapper for FragmentedMapper {
    fn set_mmap_strategy(&self, strategy: MmapStrategy) {
        self.inner().strategy.store(strategy, Ordering::Relaxed);
    }

    fn eagerly_mmap_all_spaces(&self, _space_map: &[Address]) {}

    fn mark_as_mapped(&self, mut start: Address, bytes: usize) {
        let end = start + bytes;
        // Iterate over the slabs covered
        while start < end {
            let high = if end > Self::slab_limit(start) && !Self::slab_limit(start).is_zero() {
                Self::slab_limit(start)
            } else {
                end
            };
            let slab = Self::slab_align_down(start);
            let start_chunk = Self::chunk_index(slab, start);
            let end_chunk = Self::chunk_index(slab, conversions::mmap_chunk_align_up(high));

            let mapped = self.get_or_allocate_slab_table(start);
            for entry in mapped.iter().take(end_chunk).skip(start_chunk) {
                entry.store(MapState::Mapped, Ordering::Relaxed);
            }
            start = high;
        }
    }

    fn quarantine_address_range(&self, mut start: Address, pages: usize) -> Result<()> {
        debug_assert!(start.is_aligned_to(BYTES_IN_PAGE));

        let end = start + conversions::pages_to_bytes(pages);

        // Each `MapState` entry governs a chunk.
        // Align down to the chunk start because we only mmap multiples of whole chunks.
        let mmap_start = conversions::mmap_chunk_align_down(start);

        // We collect the chunk states from slabs to process them in bulk.
        let mut state_slices = vec![];

        // Iterate over the slabs covered
        while start < end {
            let high = if end > Self::slab_limit(start) && !Self::slab_limit(start).is_zero() {
                Self::slab_limit(start)
            } else {
                end
            };

            let slab = Self::slab_align_down(start);
            let start_chunk = Self::chunk_index(slab, start);
            let end_chunk = Self::chunk_index(slab, conversions::mmap_chunk_align_up(high));

            let mapped = self.get_or_allocate_slab_table(start);
            state_slices.push(&mapped[start_chunk..end_chunk]);

            start = high;
        }

        #[cfg(debug_assertions)]
        {
            // Check if the number of entries are normal.
            let mmap_end = conversions::mmap_chunk_align_up(end);
            let num_slices = state_slices.iter().map(|s| s.len()).sum::<usize>();

            debug_assert_eq!(mmap_start + BYTES_IN_CHUNK * num_slices, mmap_end);
        }

        // Transition the chunks in bulk.
        {
            let _guard = self.lock.lock().unwrap();
            MapState::bulk_transition_to_quarantined(
                state_slices.as_slice(),
                mmap_start,
                self.inner().strategy.load(Ordering::Relaxed),
            )?;
        }

        Ok(())
    }

    fn ensure_mapped(&self, mut start: Address, pages: usize) -> Result<()> {
        let end = start + conversions::pages_to_bytes(pages);
        // Iterate over the slabs covered
        while start < end {
            let base = Self::slab_align_down(start);
            let high = if end > Self::slab_limit(start) && !Self::slab_limit(start).is_zero() {
                Self::slab_limit(start)
            } else {
                end
            };

            let slab = Self::slab_align_down(start);
            let start_chunk = Self::chunk_index(slab, start);
            let end_chunk = Self::chunk_index(slab, conversions::mmap_chunk_align_up(high));

            let mapped = self.get_or_allocate_slab_table(start);

            /* Iterate over the chunks within the slab */
            for (chunk, entry) in mapped.iter().enumerate().take(end_chunk).skip(start_chunk) {
                if matches!(entry.load(Ordering::Relaxed), MapState::Mapped) {
                    continue;
                }

                let mmap_start = Self::chunk_index_to_address(base, chunk);
                let _guard = self.lock.lock().unwrap();
                MapState::transition_to_mapped(
                    entry,
                    mmap_start,
                    self.inner().strategy.load(Ordering::Relaxed),
                )?;
            }
            start = high;
        }
        Ok(())
    }

    /**
     * Return {@code true} if the given address has been mmapped
     *
     * @param addr The address in question.
     * @return {@code true} if the given address has been mmapped
     */
    fn is_mapped_address(&self, addr: Address) -> bool {
        let mapped = self.slab_table(addr);
        match mapped {
            Some(mapped) => {
                mapped[Self::chunk_index(Self::slab_align_down(addr), addr)].load(Ordering::Relaxed)
                    == MapState::Mapped
            }
            _ => false,
        }
    }

    fn protect(&self, mut start: Address, pages: usize) {
        let end = start + conversions::pages_to_bytes(pages);
        let _guard = self.lock.lock().unwrap();
        // Iterate over the slabs covered
        while start < end {
            let base = Self::slab_align_down(start);
            let high = if end > Self::slab_limit(start) && !Self::slab_limit(start).is_zero() {
                Self::slab_limit(start)
            } else {
                end
            };

            let slab = Self::slab_align_down(start);
            let start_chunk = Self::chunk_index(slab, start);
            let end_chunk = Self::chunk_index(slab, conversions::mmap_chunk_align_up(high));

            let mapped = self.get_or_allocate_slab_table(start);

            for (chunk, entry) in mapped.iter().enumerate().take(end_chunk).skip(start_chunk) {
                let mmap_start = Self::chunk_index_to_address(base, chunk);
                MapState::transition_to_protected(entry, mmap_start).unwrap();
            }
            start = high;
        }
    }
}

impl FragmentedMapper {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            inner: UnsafeCell::new(InnerFragmentedMapper {
                free_slab_index: 0,
                free_slabs: (0..MAX_SLABS).map(|_| Some(Self::new_slab())).collect(),
                slab_table: (0..SLAB_TABLE_SIZE).map(|_| None).collect(),
                slab_map: vec![SENTINEL; SLAB_TABLE_SIZE],
                strategy: Atomic::new(MmapStrategy::Normal),
            }),
        }
    }

    fn new_slab() -> Box<Slab> {
        let mapped: Box<Slab> =
            Box::new(unsafe { transmute([MapState::Unmapped; MMAP_NUM_CHUNKS]) });
        mapped
    }

    fn hash(addr: Address) -> usize {
        let mut initial = (addr & !MMAP_SLAB_MASK) >> LOG_MMAP_SLAB_BYTES;
        let mut hash = 0;
        while initial != 0 {
            hash ^= initial & HASH_MASK;
            initial >>= LOG_SLAB_TABLE_SIZE;
        }
        hash
    }

    fn slab_table(&self, addr: Address) -> Option<&Slab> {
        self.get_or_optionally_allocate_slab_table(addr, false)
    }

    fn get_or_allocate_slab_table(&self, addr: Address) -> &Slab {
        self.get_or_optionally_allocate_slab_table(addr, true)
            .unwrap()
    }

    fn inner(&self) -> &InnerFragmentedMapper {
        unsafe { &*self.inner.get() }
    }
    #[allow(clippy::mut_from_ref)]
    fn inner_mut(&self) -> &mut InnerFragmentedMapper {
        unsafe { &mut *self.inner.get() }
    }

    fn get_or_optionally_allocate_slab_table(
        &self,
        addr: Address,
        allocate: bool,
    ) -> Option<&Slab> {
        debug_assert!(addr != SENTINEL);
        let base = unsafe { Address::from_usize(addr & !MMAP_SLAB_MASK) };
        let hash = Self::hash(base);
        let mut index = hash; // Use 'index' to iterate over the hash table so that we remember where we started
        loop {
            /* Check for a hash-table hit.  Should be the frequent case. */
            if base == self.inner().slab_map[index] {
                return self.slab_table_for(addr, index);
            }
            let _guard = self.lock.lock().unwrap();

            /* Check whether another thread has allocated a slab while we were acquiring the lock */
            if base == self.inner().slab_map[index] {
                // drop(guard);
                return self.slab_table_for(addr, index);
            }

            /* Check for a free slot */
            if self.inner().slab_map[index] == SENTINEL {
                if !allocate {
                    // drop(guard);
                    return None;
                }
                unsafe {
                    self.commit_free_slab(index);
                }
                self.inner_mut().slab_map[index] = base;
                return self.slab_table_for(addr, index);
            }
            //   lock.release();
            index += 1;
            index %= SLAB_TABLE_SIZE;
            assert!(index != hash, "MMAP slab table is full!");
        }
    }

    fn slab_table_for(&self, _addr: Address, index: usize) -> Option<&Slab> {
        debug_assert!(self.inner().slab_table[index].is_some());
        self.inner().slab_table[index].as_ref().map(|x| x as &Slab)
    }

    /**
     * Take a free slab of chunks from the freeSlabs array, and insert it
     * at the correct index in the slabTable.
     * @param index slab table index
     */
    /// # Safety
    ///
    /// Caller must ensure that only one thread is calling this function at a time.
    unsafe fn commit_free_slab(&self, index: usize) {
        assert!(
            self.inner().free_slab_index < MAX_SLABS,
            "All free slabs used: virtual address space is exhausled."
        );
        debug_assert!(self.inner().slab_table[index].is_none());
        debug_assert!(self.inner().free_slabs[self.inner().free_slab_index].is_some());
        ::std::mem::swap(
            &mut self.inner_mut().slab_table[index],
            &mut self.inner_mut().free_slabs[self.inner().free_slab_index],
        );
        self.inner_mut().free_slab_index += 1;
    }

    fn chunk_index_to_address(base: Address, chunk: usize) -> Address {
        base + (chunk << LOG_MMAP_CHUNK_BYTES)
    }

    /**
     * @param addr an address
     * @return the base address of the enclosing slab
     */
    fn slab_align_down(addr: Address) -> Address {
        unsafe { Address::from_usize(addr & !MMAP_SLAB_MASK) }
    }

    /**
     * @param addr an address
     * @return the base address of the next slab
     */
    fn slab_limit(addr: Address) -> Address {
        Self::slab_align_down(addr) + MMAP_SLAB_EXTENT
    }

    /**
     * @param slab Address of the slab
     * @param addr Address within a chunk (could be in the next slab)
     * @return The index of the chunk within the slab (could be beyond the end of the slab)
     */
    fn chunk_index(slab: Address, addr: Address) -> usize {
        let delta = addr - slab;
        delta >> LOG_MMAP_CHUNK_BYTES
    }
}

impl Default for FragmentedMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::constants::LOG_BYTES_IN_PAGE;
    use crate::util::heap::layout::vm_layout::MMAP_CHUNK_BYTES;
    use crate::util::memory;
    use crate::util::test_util::FRAGMENTED_MMAPPER_TEST_REGION;
    use crate::util::test_util::{serial_test, with_cleanup};
    use crate::util::{conversions, Address};

    const FIXED_ADDRESS: Address = FRAGMENTED_MMAPPER_TEST_REGION.start;
    const MAX_BYTES: usize = FRAGMENTED_MMAPPER_TEST_REGION.size;

    fn pages_to_chunks_up(pages: usize) -> usize {
        conversions::raw_align_up(pages, MMAP_CHUNK_BYTES) / MMAP_CHUNK_BYTES
    }

    fn get_chunk_map_state(mmapper: &FragmentedMapper, chunk: Address) -> Option<MapState> {
        assert_eq!(conversions::mmap_chunk_align_up(chunk), chunk);
        let mapped = mmapper.slab_table(chunk);
        mapped.map(|m| {
            m[FragmentedMapper::chunk_index(FragmentedMapper::slab_align_down(chunk), chunk)]
                .load(Ordering::Relaxed)
        })
    }

    #[test]
    fn address_hashing() {
        for i in 0..10 {
            unsafe {
                let a = i << LOG_MMAP_SLAB_BYTES;
                assert_eq!(FragmentedMapper::hash(Address::from_usize(a)), i);

                let b = a + ((i + 1) << (LOG_MMAP_SLAB_BYTES + LOG_SLAB_TABLE_SIZE + 1));
                assert_eq!(
                    FragmentedMapper::hash(Address::from_usize(b)),
                    i ^ ((i + 1) << 1)
                );

                let c = b + ((i + 2) << (LOG_MMAP_SLAB_BYTES + LOG_SLAB_TABLE_SIZE * 2 + 2));
                assert_eq!(
                    FragmentedMapper::hash(Address::from_usize(c)),
                    i ^ ((i + 1) << 1) ^ ((i + 2) << 2)
                );
            }
        }
    }

    #[test]
    fn ensure_mapped_1page() {
        serial_test(|| {
            let pages = 1;
            with_cleanup(
                || {
                    let mmapper = FragmentedMapper::new();
                    mmapper.ensure_mapped(FIXED_ADDRESS, pages).unwrap();

                    let chunks = pages_to_chunks_up(pages);
                    for i in 0..chunks {
                        assert_eq!(
                            get_chunk_map_state(
                                &mmapper,
                                FIXED_ADDRESS + (i << LOG_BYTES_IN_CHUNK)
                            ),
                            Some(MapState::Mapped)
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_BYTES).unwrap();
                },
            )
        })
    }
    #[test]
    fn ensure_mapped_1chunk() {
        serial_test(|| {
            let pages = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
            with_cleanup(
                || {
                    let mmapper = FragmentedMapper::new();
                    mmapper.ensure_mapped(FIXED_ADDRESS, pages).unwrap();

                    let chunks = pages_to_chunks_up(pages);
                    for i in 0..chunks {
                        assert_eq!(
                            get_chunk_map_state(
                                &mmapper,
                                FIXED_ADDRESS + (i << LOG_BYTES_IN_CHUNK)
                            ),
                            Some(MapState::Mapped)
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_BYTES).unwrap();
                },
            )
        })
    }

    #[test]
    fn ensure_mapped_more_than_1chunk() {
        serial_test(|| {
            let pages = (MMAP_CHUNK_BYTES + MMAP_CHUNK_BYTES / 2) >> LOG_BYTES_IN_PAGE as usize;
            with_cleanup(
                || {
                    let mmapper = FragmentedMapper::new();
                    mmapper.ensure_mapped(FIXED_ADDRESS, pages).unwrap();

                    let chunks = pages_to_chunks_up(pages);
                    for i in 0..chunks {
                        assert_eq!(
                            get_chunk_map_state(
                                &mmapper,
                                FIXED_ADDRESS + (i << LOG_BYTES_IN_CHUNK)
                            ),
                            Some(MapState::Mapped)
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_BYTES).unwrap();
                },
            )
        })
    }

    #[test]
    fn protect() {
        serial_test(|| {
            with_cleanup(
                || {
                    // map 2 chunks
                    let mmapper = FragmentedMapper::new();
                    let pages_per_chunk = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages_per_chunk * 2)
                        .unwrap();

                    // protect 1 chunk
                    mmapper.protect(FIXED_ADDRESS, pages_per_chunk);

                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS),
                        Some(MapState::Protected)
                    );
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS + MMAP_CHUNK_BYTES),
                        Some(MapState::Mapped)
                    );
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_BYTES).unwrap();
                },
            )
        })
    }

    #[test]
    fn ensure_mapped_on_protected_chunks() {
        serial_test(|| {
            with_cleanup(
                || {
                    // map 2 chunks
                    let mmapper = FragmentedMapper::new();
                    let pages_per_chunk = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages_per_chunk * 2)
                        .unwrap();

                    // protect 1 chunk
                    mmapper.protect(FIXED_ADDRESS, pages_per_chunk);

                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS),
                        Some(MapState::Protected)
                    );
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS + MMAP_CHUNK_BYTES),
                        Some(MapState::Mapped)
                    );

                    // ensure mapped - this will unprotect the previously protected chunk
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages_per_chunk * 2)
                        .unwrap();
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS),
                        Some(MapState::Mapped)
                    );
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS + MMAP_CHUNK_BYTES),
                        Some(MapState::Mapped)
                    );
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_BYTES).unwrap();
                },
            )
        })
    }
}
