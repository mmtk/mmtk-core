use super::mmapper::MapState;
use super::Mmapper;
use crate::util::memory::MmapAnnotation;
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

const MMAP_NUM_CHUNKS: usize = if LOG_BYTES_IN_ADDRESS_SPACE == 32 {
    1 << (LOG_BYTES_IN_ADDRESS_SPACE as usize - LOG_MMAP_CHUNK_BYTES)
} else {
    1 << (33 - LOG_MMAP_CHUNK_BYTES)
};
pub const VERBOSE: bool = true;

pub struct ByteMapMmapper {
    lock: Mutex<()>,
    mapped: [Atomic<MapState>; MMAP_NUM_CHUNKS],
}

impl fmt::Debug for ByteMapMmapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ByteMapMmapper({})", MMAP_NUM_CHUNKS)
    }
}

impl Mmapper for ByteMapMmapper {
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

    fn ensure_mapped(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
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
            MapState::transition_to_mapped(&self.mapped[chunk], mmap_start, strategy, anno)
                .unwrap();
        }

        Ok(())
    }

    fn quarantine_address_range(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
        let start_chunk = Self::address_to_mmap_chunks_down(start);
        let end_chunk = Self::address_to_mmap_chunks_up(start + pages_to_bytes(pages));
        trace!(
            "Calling quarantine_address_range with start={:?} and {} pages, {}-{}",
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
            MapState::transition_to_quarantined(&self.mapped[chunk], mmap_start, strategy, anno)
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
        // Because AtomicU8 does not implement Copy, it is a compilation error to usen the
        // expression `[Atomic::new(MapState::Unmapped); MMAP_NUM_CHUNKS]` because that involves
        // copying.  We must define a constant for it.
        //
        // TODO: Use the inline const expression `const { Atomic::new(MapState::Unmapped) }` after
        // we bump MSRV to 1.79.

        // If we declare a const Atomic, Clippy will warn about const items being interior mutable.
        // Using inline const expression will eliminate this warning, but that is experimental until
        // 1.79.  Fix it after we bump MSRV.
        #[allow(clippy::declare_interior_mutable_const)]
        const INITIAL_ENTRY: Atomic<MapState> = Atomic::new(MapState::Unmapped);

        ByteMapMmapper {
            lock: Mutex::new(()),
            mapped: [INITIAL_ENTRY; MMAP_NUM_CHUNKS],
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
    use crate::mmap_anno_test;
    use crate::util::heap::layout::Mmapper;
    use crate::util::Address;

    use crate::util::constants::LOG_BYTES_IN_PAGE;
    use crate::util::conversions::pages_to_bytes;
    use crate::util::heap::layout::mmapper::MapState;
    use crate::util::heap::layout::vm_layout::MMAP_CHUNK_BYTES;
    use crate::util::memory::{self, MmapStrategy};
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
            let pages = 1;
            let start_chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
            let end_chunk =
                ByteMapMmapper::address_to_mmap_chunks_up(FIXED_ADDRESS + pages_to_bytes(pages));
            let test_memory_bytes = (end_chunk - start_chunk) * MMAP_CHUNK_BYTES;
            with_cleanup(
                || {
                    let mmapper = ByteMapMmapper::new();
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages, MmapStrategy::TEST, mmap_anno_test!())
                        .unwrap();

                    for chunk in start_chunk..end_chunk {
                        assert_eq!(
                            mmapper.mapped[chunk].load(Ordering::Relaxed),
                            MapState::Mapped
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, test_memory_bytes).unwrap();
                },
            )
        })
    }

    #[test]
    fn ensure_mapped_1chunk() {
        serial_test(|| {
            let pages = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
            let start_chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
            let end_chunk =
                ByteMapMmapper::address_to_mmap_chunks_up(FIXED_ADDRESS + pages_to_bytes(pages));
            let test_memory_bytes = (end_chunk - start_chunk) * MMAP_CHUNK_BYTES;
            with_cleanup(
                || {
                    let mmapper = ByteMapMmapper::new();
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages, MmapStrategy::TEST, mmap_anno_test!())
                        .unwrap();

                    for chunk in start_chunk..end_chunk {
                        assert_eq!(
                            mmapper.mapped[chunk].load(Ordering::Relaxed),
                            MapState::Mapped
                        );
                    }
                },
                || {
                    memory::munmap(FIXED_ADDRESS, test_memory_bytes).unwrap();
                },
            )
        })
    }

    #[test]
    fn ensure_mapped_more_than_1chunk() {
        serial_test(|| {
            let pages = (MMAP_CHUNK_BYTES + MMAP_CHUNK_BYTES / 2) >> LOG_BYTES_IN_PAGE as usize;
            let start_chunk = ByteMapMmapper::address_to_mmap_chunks_down(FIXED_ADDRESS);
            let end_chunk =
                ByteMapMmapper::address_to_mmap_chunks_up(FIXED_ADDRESS + pages_to_bytes(pages));
            let test_memory_bytes = (end_chunk - start_chunk) * MMAP_CHUNK_BYTES;
            with_cleanup(
                || {
                    let mmapper = ByteMapMmapper::new();
                    mmapper
                        .ensure_mapped(FIXED_ADDRESS, pages, MmapStrategy::TEST, mmap_anno_test!())
                        .unwrap();

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
                    memory::munmap(FIXED_ADDRESS, test_memory_bytes).unwrap();
                },
            )
        })
    }

    #[test]
    fn protect() {
        serial_test(|| {
            let test_memory_bytes = MMAP_CHUNK_BYTES * 2;
            let test_memory_pages = test_memory_bytes >> LOG_BYTES_IN_PAGE;
            let protect_memory_bytes = MMAP_CHUNK_BYTES;
            let protect_memory_pages = protect_memory_bytes >> LOG_BYTES_IN_PAGE;
            with_cleanup(
                || {
                    // map 2 chunks
                    let mmapper = ByteMapMmapper::new();
                    mmapper
                        .ensure_mapped(
                            FIXED_ADDRESS,
                            test_memory_pages,
                            MmapStrategy::TEST,
                            mmap_anno_test!(),
                        )
                        .unwrap();

                    // protect 1 chunk
                    mmapper.protect(FIXED_ADDRESS, protect_memory_pages);

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
                    memory::munmap(FIXED_ADDRESS, test_memory_bytes).unwrap();
                },
            )
        })
    }

    #[test]
    fn ensure_mapped_on_protected_chunks() {
        serial_test(|| {
            let test_memory_bytes = MMAP_CHUNK_BYTES * 2;
            let test_memory_pages = test_memory_bytes >> LOG_BYTES_IN_PAGE;
            let protect_memory_pages_1 = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE; // protect one chunk in the first protect
            let protect_memory_pages_2 = test_memory_pages; // protect both chunks in the second protect
            with_cleanup(
                || {
                    // map 2 chunks
                    let mmapper = ByteMapMmapper::new();
                    mmapper
                        .ensure_mapped(
                            FIXED_ADDRESS,
                            test_memory_pages,
                            MmapStrategy::TEST,
                            mmap_anno_test!(),
                        )
                        .unwrap();

                    // protect 1 chunk
                    mmapper.protect(FIXED_ADDRESS, protect_memory_pages_1);

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
                        .ensure_mapped(
                            FIXED_ADDRESS,
                            protect_memory_pages_2,
                            MmapStrategy::TEST,
                            mmap_anno_test!(),
                        )
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
                    memory::munmap(FIXED_ADDRESS, test_memory_bytes).unwrap();
                },
            )
        })
    }
}
