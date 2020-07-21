use super::Mmapper;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::Address;
use atomic::{Atomic, Ordering};
use std::fmt;
use std::mem::transmute;
use std::sync::Mutex;

#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq)]
enum MapState {
    Unmapped,
    Mapped,
    Protected,
}

const MMAP_NUM_CHUNKS: usize = 1 << (33 - LOG_MMAP_CHUNK_BYTES);

const LOG_MAPPABLE_BYTES: usize = 36; // 128GB - physical memory larger than this is uncommon
                                      /*
                                       * Size of a slab.  The value 10 gives a slab size of 1GB, with 1024
                                       * chunks per slab, ie a 1k slab map.  In a 64-bit address space, this
                                       * will require 1M of slab maps.
                                       */
const LOG_MMAP_CHUNKS_PER_SLAB: usize = 10;
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
    free_slab_index: usize,
    free_slabs: Vec<Option<Box<Slab>>>,
    slab_table: Vec<Option<Box<Slab>>>,
    slab_map: Vec<Address>,
}

impl fmt::Debug for FragmentedMapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FragmentedMapper({})", MMAP_NUM_CHUNKS)
    }
}

impl Mmapper for FragmentedMapper {
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

    fn ensure_mapped(&self, mut start: Address, pages: usize) {
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
                match entry.load(Ordering::Relaxed) {
                    MapState::Mapped => continue,
                    MapState::Unmapped => {
                        let mmap_start = Self::chunk_index_to_address(base, chunk);
                        let _guard = self.lock.lock().unwrap();
                        crate::util::memory::dzmmap(mmap_start, MMAP_CHUNK_BYTES).unwrap();
                    }
                    MapState::Protected => {
                        let mmap_start = Self::chunk_index_to_address(base, chunk);
                        let _guard = self.lock.lock().unwrap();
                        crate::util::memory::munprotect(mmap_start, MMAP_CHUNK_BYTES).unwrap();
                    }
                }
                entry.store(MapState::Mapped, Ordering::Relaxed);
            }
            start = high;
        }
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
                if entry.load(Ordering::Relaxed) == MapState::Mapped {
                    let mmap_start = Self::chunk_index_to_address(base, chunk);
                    crate::util::memory::mprotect(mmap_start, MMAP_CHUNK_BYTES).unwrap();
                    entry.store(MapState::Protected, Ordering::Relaxed);
                } else {
                    debug_assert!(entry.load(Ordering::Relaxed) == MapState::Protected);
                }
            }
            start = high;
        }
    }
}

impl FragmentedMapper {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            free_slab_index: 0,
            free_slabs: (0..MAX_SLABS).map(|_| Some(Self::new_slab())).collect(),
            slab_table: (0..SLAB_TABLE_SIZE).map(|_| None).collect(),
            slab_map: vec![SENTINEL; SLAB_TABLE_SIZE],
        }
    }

    fn new_slab() -> Box<Slab> {
        let mapped: Box<Slab> = box unsafe { transmute([MapState::Unmapped; MMAP_NUM_CHUNKS]) };
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
        unsafe { self.mut_self() }.get_or_optionally_allocate_slab_table(addr, false)
    }

    fn get_or_allocate_slab_table(&self, addr: Address) -> &Slab {
        unsafe { self.mut_self() }
            .get_or_optionally_allocate_slab_table(addr, true)
            .unwrap()
    }

    #[allow(clippy::cast_ref_to_mut)]
    #[allow(clippy::mut_from_ref)]
    unsafe fn mut_self(&self) -> &mut Self {
        &mut *(self as *const _ as *mut _)
    }

    fn get_or_optionally_allocate_slab_table(
        &mut self,
        addr: Address,
        allocate: bool,
    ) -> Option<&Slab> {
        debug_assert!(addr != SENTINEL);
        let base = unsafe { Address::from_usize(addr & !MMAP_SLAB_MASK) };
        let hash = Self::hash(base);
        let mut index = hash; // Use 'index' to iterate over the hash table so that we remember where we started
        loop {
            /* Check for a hash-table hit.  Should be the frequent case. */
            if base == self.slab_map[index] {
                return self.slab_table_for(addr, index);
            }
            let _guard = self.lock.lock().unwrap();

            /* Check whether another thread has allocated a slab while we were acquiring the lock */
            if base == self.slab_map[index] {
                // drop(guard);
                return self.slab_table_for(addr, index);
            }

            /* Check for a free slot */
            if self.slab_map[index] == SENTINEL {
                if !allocate {
                    // drop(guard);
                    return None;
                }
                unsafe { self.mut_self() }.commit_free_slab(index);
                self.slab_map[index] = base;
                return self.slab_table_for(addr, index);
            }
            //   lock.release();
            index += 1;
            index %= SLAB_TABLE_SIZE;
            assert!(index != hash, "MMAP slab table is full!");
        }
    }

    fn slab_table_for(&self, _addr: Address, index: usize) -> Option<&Slab> {
        debug_assert!(self.slab_table[index].is_some());
        self.slab_table[index].as_ref().map(|x| &x as &Slab)
    }

    /**
     * Take a free slab of chunks from the freeSlabs array, and insert it
     * at the correct index in the slabTable.
     * @param index slab table index
     */
    fn commit_free_slab(&mut self, index: usize) {
        assert!(
            self.free_slab_index < MAX_SLABS,
            "All free slabs used: virtual address space is exhausled."
        );
        debug_assert!(self.slab_table[index].is_none());
        debug_assert!(self.free_slabs[self.free_slab_index].is_some());
        ::std::mem::swap(
            &mut self.slab_table[index],
            &mut self.free_slabs[self.free_slab_index],
        );
        self.free_slab_index += 1;
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
