use super::mmapper::MapState;
use super::Mmapper;
use crate::util::constants::BYTES_IN_PAGE;
use crate::util::conversions;
use crate::util::heap::layout::vm_layout::*;
use crate::util::memory::{MmapAnnotation, MmapStrategy};
use crate::util::rust_util::atomic_box::OnceOptionBox;
use crate::util::Address;
use atomic::{Atomic, Ordering};
use std::fmt;
use std::io::Result;
use std::sync::Mutex;

/// Logarithm of the address space size a user-space program is allowed to use.
/// This is enough for ARM64, x86_64 and some other architectures.
/// Feel free to increase it if we plan to support larger address spaces.
const LOG_MAPPABLE_BYTES: usize = 48;
/// Address space size a user-space program is allowed to use.
const MAPPABLE_BYTES: usize = 1 << LOG_MAPPABLE_BYTES;

/// Log number of bytes per slab.
/// For a two-level array, it is advisable to choose the arithmetic mean of [`LOG_MAPPABLE_BYTES`]
/// and [`LOG_MMAP_CHUNK_BYTES`] in order to make [`MMAP_SLAB_BYTES`] the geometric mean of
/// [`MAPPABLE_BYTES`] and [`MMAP_CHUNK_BYTES`].  This will balance the array size of
/// [`TwoLevelMmapper::slabs`] and [`Slab`].
///
/// TODO: Use `usize::midpoint` after bumping MSRV to 1.85
const LOG_MMAP_SLAB_BYTES: usize = LOG_MMAP_CHUNK_BYTES + (LOG_MAPPABLE_BYTES - LOG_MMAP_CHUNK_BYTES) / 2;
/// Number of bytes per slab.
const MMAP_SLAB_BYTES: usize = 1 << LOG_MMAP_SLAB_BYTES;

/// Log number of chunks per slab.
const LOG_MMAP_CHUNKS_PER_SLAB: usize = LOG_MMAP_SLAB_BYTES - LOG_MMAP_CHUNK_BYTES;
/// Number of chunks per slab.
const MMAP_CHUNKS_PER_SLAB: usize = 1 << LOG_MMAP_CHUNKS_PER_SLAB;

/// Mask for getting in-slab bits from an address.
/// Invert this to get out-of-slab bits.
const MMAP_SLAB_MASK: usize = (1 << LOG_MMAP_SLAB_BYTES) - 1;

/// Logarithm of maximum number of slabs, which determines the maximum mappable address space.
const LOG_MAX_SLABS: usize = LOG_MAPPABLE_BYTES - LOG_MMAP_SLAB_BYTES;
/// maximum number of slabs, which determines the maximum mappable address space.
const MAX_SLABS: usize = 1 << LOG_MAX_SLABS;

/// The slab type.  Each slab holds the `MapState` of multiple chunks.
type Slab = [Atomic<MapState>; 1 << LOG_MMAP_CHUNKS_PER_SLAB];

/// A two-level implementation of `Mmapper`.
pub struct TwoLevelMmapper {
    /// Lock for transitioning map states.
    ///
    /// FIXME: We only needs the lock when transitioning map states.
    /// The `TwoLevelMmapper` itself is completely lock-free even when allocating new slabs.
    /// We should move the lock one leve above, to `MapState`.
    transition_lock: Mutex<()>,
    /// Slabs
    slabs: Vec<OnceOptionBox<Slab>>,
}

unsafe impl Send for TwoLevelMmapper {}
unsafe impl Sync for TwoLevelMmapper {}

impl fmt::Debug for TwoLevelMmapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TwoLevelMapper({})", 1 << LOG_MAX_SLABS)
    }
}

impl Mmapper for TwoLevelMmapper {
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

    fn quarantine_address_range(
        &self,
        mut start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
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
            let _guard = self.transition_lock.lock().unwrap();
            MapState::bulk_transition_to_quarantined(
                state_slices.as_slice(),
                mmap_start,
                strategy,
                anno,
            )?;
        }

        Ok(())
    }

    fn ensure_mapped(
        &self,
        mut start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
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
                let _guard = self.transition_lock.lock().unwrap();
                MapState::transition_to_mapped(entry, mmap_start, strategy, anno)?;
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
        let _guard = self.transition_lock.lock().unwrap();
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

impl TwoLevelMmapper {
    pub fn new() -> Self {
        Self {
            transition_lock: Default::default(),
            slabs: unsafe { crate::util::rust_util::zeroed_alloc::new_zeroed_vec(MAX_SLABS) },
        }
    }

    fn new_slab() -> Slab {
        std::array::from_fn(|_| Atomic::new(MapState::Unmapped))
    }

    fn slab_table(&self, addr: Address) -> Option<&Slab> {
        self.get_or_optionally_allocate_slab_table(addr, false)
    }

    fn get_or_allocate_slab_table(&self, addr: Address) -> &Slab {
        self.get_or_optionally_allocate_slab_table(addr, true)
            .unwrap()
    }

    fn get_or_optionally_allocate_slab_table(
        &self,
        addr: Address,
        allocate: bool,
    ) -> Option<&Slab> {
        let index = addr >> LOG_MMAP_SLAB_BYTES;
        if index > self.slabs.len() {
            panic!("addr: {addr}, index: {index}, slabs.len: {sl}", sl = self.slabs.len());
        }
        let slot = &self.slabs[index];
        if allocate {
            slot.get_or_init(Ordering::Acquire, Ordering::Release, Self::new_slab)
        } else {
            slot.get(Ordering::Acquire)
        }
    }

    fn chunk_index_to_address(base: Address, chunk: usize) -> Address {
        base + (chunk << LOG_MMAP_CHUNK_BYTES)
    }

    /// Align `addr` down to slab size.
    fn slab_align_down(addr: Address) -> Address {
        addr.align_down(MMAP_SLAB_BYTES)
    }

    /// Get the base address of the next slab after the slab that contains `addr`.
    fn slab_limit(addr: Address) -> Address {
        Self::slab_align_down(addr) + MMAP_SLAB_BYTES
    }

    /// Return the index of the chunk that contains `addr` within the slab starting at `slab`
    /// If `addr` is beyond the end of the slab, the result could be beyond the end of the slab.
    fn chunk_index(slab: Address, addr: Address) -> usize {
        let delta = addr - slab;
        delta >> LOG_MMAP_CHUNK_BYTES
    }
}

impl Default for TwoLevelMmapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmap_anno_test;
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

    fn get_chunk_map_state(mmapper: &TwoLevelMmapper, chunk: Address) -> Option<MapState> {
        assert_eq!(conversions::mmap_chunk_align_up(chunk), chunk);
        let mapped = mmapper.slab_table(chunk);
        mapped.map(|m| {
            m[TwoLevelMmapper::chunk_index(TwoLevelMmapper::slab_align_down(chunk), chunk)]
                .load(Ordering::Relaxed)
        })
    }

    #[test]
    fn ensure_mapped_1page() {
        serial_test(|| {
            let pages = 1;
            with_cleanup(
                || {
                    let mmapper = TwoLevelMmapper::new();
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages, MmapStrategy::TEST, mmap_anno_test!())
                        .unwrap();

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
                    let mmapper = TwoLevelMmapper::new();
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages, MmapStrategy::TEST, mmap_anno_test!())
                        .unwrap();

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
                    let mmapper = TwoLevelMmapper::new();
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages, MmapStrategy::TEST, mmap_anno_test!())
                        .unwrap();

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
                    let mmapper = TwoLevelMmapper::new();
                    let pages_per_chunk = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
                    mmapper
                        .ensure_mapped(
                            FIXED_ADDRESS,
                            pages_per_chunk * 2,
                            MmapStrategy::TEST,
                            mmap_anno_test!(),
                        )
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
                    let mmapper = TwoLevelMmapper::new();
                    let pages_per_chunk = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
                    mmapper
                        .ensure_mapped(
                            FIXED_ADDRESS,
                            pages_per_chunk * 2,
                            MmapStrategy::TEST,
                            mmap_anno_test!(),
                        )
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
                        .ensure_mapped(
                            FIXED_ADDRESS,
                            pages_per_chunk * 2,
                            MmapStrategy::TEST,
                            mmap_anno_test!(),
                        )
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
