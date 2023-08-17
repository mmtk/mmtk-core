use super::mmapper::MapState;
use super::Mmapper;
use crate::util::Address;

use crate::util::constants::*;
use crate::util::conversions::pages_to_bytes;
use crate::util::heap::layout::vm_layout::*;
use crate::util::memory::MmapStrategy;
use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use atomic::Atomic;
use std::io::Result;
use std::mem::transmute;

const MMAP_NUM_CHUNKS: usize = if LOG_BYTES_IN_ADDRESS_SPACE == 32 {
    1 << (LOG_BYTES_IN_ADDRESS_SPACE as usize - LOG_MMAP_CHUNK_BYTES)
} else {
    1 << (33 - LOG_MMAP_CHUNK_BYTES)
};
pub const VERBOSE: bool = true;

pub struct ByteMapMmapper {
    lock: Mutex<()>,
    mapped: [Atomic<MapState>; MMAP_NUM_CHUNKS],
    strategy: Atomic<MmapStrategy>,
}

impl fmt::Debug for ByteMapMmapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ByteMapMmapper({})", MMAP_NUM_CHUNKS)
    }
}

impl Mmapper for ByteMapMmapper {
    fn set_mmap_strategy(&self, strategy: MmapStrategy) {
        self.strategy.store(strategy, Ordering::Relaxed);
    }

    fn eagerly_mmap_all_spaces(&self, _space_map: &[Address]) {
        unimplemented!()
    }

    fn mark_as_mapped(&self, start: Address, bytes: usize) {
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let end_chunk = Self::address_to_mmap_chunks_up(start + bytes) - 1;
        for i in start_chunk..=end_chunk {
            self.mapped[i].store(MapState::Mapped, Ordering::Relaxed);
        }
    }

    fn ensure_mapped(&self, start: Address, pages: usize) -> Result<()> {
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let end_chunk = Self::address_to_mmap_chunks_up(start + pages_to_bytes(pages));
        trace!(
            "Calling ensure_mapped with start={:?} and {} pages, {}-{}",
            start,
            pages,
            Self::mmap_chunks_to_address(start_chunk),
            Self::mmap_chunks_to_address(end_chunk)
        );

        for chunk in start_chunk..end_chunk {
            if self.mapped[chunk].load(Ordering::Relaxed) == MapState::Mapped {
                continue;
            }

            let mmap_start = Self::mmap_chunks_to_address(chunk);
            let _guard = self.lock.lock().unwrap();
            MapState::transition_to_mapped(
                &self.mapped[chunk],
                mmap_start,
                self.strategy.load(Ordering::Relaxed),
            )
            .unwrap();
        }

        Ok(())
    }

    fn quarantine_address_range(&self, start: Address, pages: usize) -> Result<()> {
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let end_chunk = Self::address_to_mmap_chunks_up(start + pages_to_bytes(pages));
        trace!(
            "Calling quanrantine_address_range with start={:?} and {} pages, {}-{}",
            start,
            pages,
            Self::mmap_chunks_to_address(start_chunk),
            Self::mmap_chunks_to_address(end_chunk)
        );

        for chunk in start_chunk..end_chunk {
            if self.mapped[chunk].load(Ordering::Relaxed) == MapState::Mapped {
                continue;
            }

            let mmap_start = Self::mmap_chunks_to_address(chunk);
            let _guard = self.lock.lock().unwrap();
            MapState::transition_to_quarantined(
                &self.mapped[chunk],
                mmap_start,
                self.strategy.load(Ordering::Relaxed),
            )
            .unwrap();
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
        let chunk = Self::address_to_mmap_chunks_down(addr);
        self.mapped[chunk].load(Ordering::Relaxed) == MapState::Mapped
    }

    fn protect(&self, start: Address, pages: usize) {
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let chunks = Self::pages_to_mmap_chunks_up(pages);
        let end_chunk = start_chunk + chunks;
        let _guard = self.lock.lock().unwrap();

        for chunk in start_chunk..end_chunk {
            let mmap_start = Self::mmap_chunks_to_address(chunk);
            MapState::transition_to_protected(&self.mapped[chunk], mmap_start).unwrap();
        }
    }
}

impl ByteMapMmapper {
    pub fn new() -> Self {
        // Hacky because AtomicU8 does not implement Copy
        // Should be fiiine because AtomicXXX has the same bit representation as XXX
        ByteMapMmapper {
            lock: Mutex::new(()),
            mapped: unsafe { transmute([MapState::Unmapped; MMAP_NUM_CHUNKS]) },
            strategy: Atomic::new(MmapStrategy::Normal),
        }
    }

    fn bytes_to_mmap_chunks_up(bytes: usize) -> usize {
        (bytes + MMAP_CHUNK_BYTES - 1) >> LOG_MMAP_CHUNK_BYTES
    }

    fn pages_to_mmap_chunks_up(pages: usize) -> usize {
        Self::bytes_to_mmap_chunks_up(pages_to_bytes(pages))
    }

    fn address_to_mmap_chunks_down(addr: Address) -> usize {
        addr >> LOG_MMAP_CHUNK_BYTES
    }

    fn mmap_chunks_to_address(chunk: usize) -> Address {
        unsafe { Address::from_usize(chunk << LOG_MMAP_CHUNK_BYTES) }
    }

    fn address_to_mmap_chunks_up(addr: Address) -> usize {
        (addr + MMAP_CHUNK_BYTES - 1) >> LOG_MMAP_CHUNK_BYTES
    }
}

impl Default for ByteMapMmapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ByteMapMmapper;
    use crate::util::heap::layout::Mmapper;
    use crate::util::Address;

    use crate::util::constants::LOG_BYTES_IN_PAGE;
    use crate::util::conversions::pages_to_bytes;
    use crate::util::heap::layout::mmapper::MapState;
    use crate::util::heap::layout::vm_layout::MMAP_CHUNK_BYTES;
    use crate::util::memory;
    use crate::util::test_util::BYTE_MAP_MMAPPER_TEST_REGION;
    use crate::util::test_util::{serial_test, with_cleanup};
    use std::sync::atomic::Ordering;

    const CHUNK_SIZE: usize = 1 << 22;
    const FIXED_ADDRESS: Address = BYTE_MAP_MMAPPER_TEST_REGION.start;
    const MAX_SIZE: usize = BYTE_MAP_MMAPPER_TEST_REGION.size;

    #[test]
    fn address_to_mmap_chunks() {
        for i in 0..10 {
            unsafe {
                let start = CHUNK_SIZE * i;
                assert_eq!(
                    ByteMapMmapper::address_to_mmap_chunks_up(Address::from_usize(start)),
                    i
                );
                assert_eq!(
                    ByteMapMmapper::address_to_mmap_chunks_down(Address::from_usize(start)),
                    i
                );

                let middle = start + 8;
                assert_eq!(
                    ByteMapMmapper::address_to_mmap_chunks_up(Address::from_usize(middle)),
                    i + 1
                );
                assert_eq!(
                    ByteMapMmapper::address_to_mmap_chunks_down(Address::from_usize(middle)),
                    i
                );

                let end = start + CHUNK_SIZE;
                assert_eq!(
                    ByteMapMmapper::address_to_mmap_chunks_up(Address::from_usize(end)),
                    i + 1
                );
                assert_eq!(
                    ByteMapMmapper::address_to_mmap_chunks_down(Address::from_usize(end)),
                    i + 1
                );
            }
        }
    }

    #[test]
    fn ensure_mapped_1page() {
        serial_test(|| {
            with_cleanup(
                || {
                    let mmapper = ByteMapMmapper::new();
                    let pages = 1;
                    mmapper.ensure_mapped(FIXED_ADDRESS, pages).unwrap();

                    let start_chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
                    let end_chunk = ByteMapMmapper::address_to_mmap_chunks_up(
                        FIXED_ADDRESS + pages_to_bytes(pages),
                    );
                    for chunk in start_chunk..end_chunk {
                        assert_eq!(
                            mmapper.mapped[chunk].load(Ordering::Relaxed),
                            MapState::Mapped
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_SIZE).unwrap();
                },
            )
        })
    }

    #[test]
    fn ensure_mapped_1chunk() {
        serial_test(|| {
            with_cleanup(
                || {
                    let mmapper = ByteMapMmapper::new();
                    let pages = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
                    mmapper.ensure_mapped(FIXED_ADDRESS, pages).unwrap();

                    let start_chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
                    let end_chunk = ByteMapMmapper::address_to_mmap_chunks_up(
                        FIXED_ADDRESS + pages_to_bytes(pages),
                    );
                    for chunk in start_chunk..end_chunk {
                        assert_eq!(
                            mmapper.mapped[chunk].load(Ordering::Relaxed),
                            MapState::Mapped
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_SIZE).unwrap();
                },
            )
        })
    }

    #[test]
    fn ensure_mapped_more_than_1chunk() {
        serial_test(|| {
            with_cleanup(
                || {
                    let mmapper = ByteMapMmapper::new();
                    let pages =
                        (MMAP_CHUNK_BYTES + MMAP_CHUNK_BYTES / 2) >> LOG_BYTES_IN_PAGE as usize;
                    mmapper.ensure_mapped(FIXED_ADDRESS, pages).unwrap();

                    let start_chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
                    let end_chunk = ByteMapMmapper::address_to_mmap_chunks_up(
                        FIXED_ADDRESS + pages_to_bytes(pages),
                    );
                    assert_eq!(end_chunk - start_chunk, 2);
                    for chunk in start_chunk..end_chunk {
                        assert_eq!(
                            mmapper.mapped[chunk].load(Ordering::Relaxed),
                            MapState::Mapped
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_SIZE).unwrap();
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
                    let mmapper = ByteMapMmapper::new();
                    let pages_per_chunk = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages_per_chunk * 2)
                        .unwrap();

                    // protect 1 chunk
                    mmapper.protect(FIXED_ADDRESS, pages_per_chunk);

                    let chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
                    assert_eq!(
                        mmapper.mapped[chunk].load(Ordering::Relaxed),
                        MapState::Protected
                    );
                    assert_eq!(
                        mmapper.mapped[chunk + 1].load(Ordering::Relaxed),
                        MapState::Mapped
                    );
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_SIZE).unwrap();
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
                    let mmapper = ByteMapMmapper::new();
                    let pages_per_chunk = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages_per_chunk * 2)
                        .unwrap();

                    // protect 1 chunk
                    mmapper.protect(FIXED_ADDRESS, pages_per_chunk);

                    let chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
                    assert_eq!(
                        mmapper.mapped[chunk].load(Ordering::Relaxed),
                        MapState::Protected
                    );
                    assert_eq!(
                        mmapper.mapped[chunk + 1].load(Ordering::Relaxed),
                        MapState::Mapped
                    );

                    // ensure mapped - this will unprotect the previously protected chunk
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages_per_chunk * 2)
                        .unwrap();
                    assert_eq!(
                        mmapper.mapped[chunk].load(Ordering::Relaxed),
                        MapState::Mapped
                    );
                    assert_eq!(
                        mmapper.mapped[chunk + 1].load(Ordering::Relaxed),
                        MapState::Mapped
                    );
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_SIZE).unwrap();
                },
            )
        })
    }
}
