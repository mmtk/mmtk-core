use crate::util::heap::layout::Mmapper;
use crate::util::memory::*;
use crate::util::Address;
use crate::util::{constants::BYTES_IN_PAGE, heap::layout::vm_layout::*};
use bytemuck::NoUninit;
use std::{io::Result, sync::Mutex};

mod byte_map_storage;
#[cfg(target_pointer_width = "64")]
mod two_level_storage;

#[cfg(target_pointer_width = "32")]
type ChosenMapStateStorage = byte_map_storage::ByteMapStateStorage;
#[cfg(target_pointer_width = "64")]
type ChosenMapStateStorage = two_level_storage::TwoLevelStateStorage;

/// The back-end storage of [`ChunkStateMmapper`].  It is responsible for holding the states of each
/// chunk (eagerly or lazily) and transitioning the states in bulk.
trait MapStateStorage {
    fn get_state(&self, chunk: Address) -> Option<MapState>;
    fn bulk_set_state(&self, start: Address, bytes: usize, state: MapState);
    fn bulk_transition_state<F>(&self, start: Address, bytes: usize, transformer: F) -> Result<()>
    where
        F: FnMut(Address, usize, MapState) -> Result<Option<MapState>>;
}

/// A [`Mmapper`] implementation based on a logical array of chunk states.
///
/// The [`ChunkStateMmapper::storage`] field holds the state of each chunk, and the
/// [`ChunkStateMmapper`] itself actually makes system calls to manage the memory mapping.
///
/// As the name suggests, this implementation of [`Mmapper`] operates at the granularity of chunks.
pub struct ChunkStateMmapper {
    /// Lock for transitioning map states.
    transition_lock: Mutex<()>,
    /// This holds the [`MapState`] for each chunk.
    storage: ChosenMapStateStorage,
}

impl ChunkStateMmapper {
    pub fn new() -> Self {
        Self {
            transition_lock: Default::default(),
            storage: ChosenMapStateStorage::new(),
        }
    }

    #[cfg(test)]
    fn get_state(&self, chunk: Address) -> Option<MapState> {
        self.storage.get_state(chunk)
    }
}

/// Generic mmap and protection functionality
impl Mmapper for ChunkStateMmapper {
    /// Given an address array describing the regions of virtual memory to be used
    /// by MMTk, demand zero map all of them if they are not already mapped.
    ///
    /// Arguments:
    /// * `spaceMap`: An address array containing a pairs of start and end
    ///   addresses for each of the regions to be mapped
    fn eagerly_mmap_all_spaces(&self, _space_map: &[Address]) {
        unimplemented!()
    }

    /// Mark a number of pages as mapped, without making any
    /// request to the operating system.  Used to mark pages
    /// that the VM has already mapped.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be mapped
    /// * `bytes`: Number of bytes to ensure mapped
    fn mark_as_mapped(&self, start: Address, bytes: usize) {
        let _guard = self.transition_lock.lock().unwrap();

        let chunk_start: Address = start.align_down(BYTES_IN_CHUNK);
        let chunk_end = (start + bytes).align_up(BYTES_IN_CHUNK);
        let aligned_bytes = chunk_end - chunk_start;
        self.storage
            .bulk_set_state(chunk_start, aligned_bytes, MapState::Mapped);
    }

    /// Quarantine/reserve address range. We mmap from the OS with no reserve and with PROT_NONE,
    /// which should be little overhead. This ensures that we can reserve certain address range that
    /// we can use if needed. Quarantined memory needs to be mapped before it can be used.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be quarantined
    /// * `bytes`: Number of bytes to quarantine from the start
    /// * `strategy`: The mmap strategy.  The `prot` field is ignored because we always use
    ///   `PROT_NONE`.
    /// * `anno`: Human-readable annotation to apply to newly mapped memory ranges.
    fn quarantine_address_range(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
        let _guard = self.transition_lock.lock().unwrap();

        let chunk_start: Address = start.align_down(BYTES_IN_CHUNK);
        let chunk_end = (start + pages * BYTES_IN_PAGE).align_up(BYTES_IN_CHUNK);
        let aligned_bytes = chunk_end - chunk_start;
        self.storage.bulk_transition_state(
            chunk_start,
            aligned_bytes,
            |group_start, group_bytes, state| {
                let group_end = group_start + group_bytes;

                match state {
                    MapState::Unmapped => {
                        trace!("Trying to quarantine {} - {}", group_start, group_end);
                        mmap_noreserve(group_start, group_bytes, strategy, anno)?;
                        Ok(Some(MapState::Quarantined))
                    }
                    MapState::Quarantined => {
                        trace!("Already quarantine {} - {}", group_start, group_end);
                        Ok(None)
                    }
                    MapState::Mapped => {
                        trace!("Already mapped {} - {}", group_start, group_end);
                        Ok(None)
                    }
                    MapState::Protected => {
                        panic!("Cannot quarantine protected memory")
                    }
                }
            },
        )
    }

    /// Ensure that a range of pages is mmapped (or equivalent).  If the
    /// pages are not yet mapped, demand-zero map them. Note that mapping
    /// occurs at chunk granularity, not page granularity.
    ///
    /// Arguments:
    /// * `start`: The start of the range to be mapped.
    /// * `pages`: The size of the range to be mapped, in pages
    /// * `strategy`: The mmap strategy.
    /// * `anno`: Human-readable annotation to apply to newly mapped memory ranges.
    // NOTE: There is a monotonicity assumption so that only updates require lock
    // acquisition.
    // TODO: Fix the above to support unmapping.
    fn ensure_mapped(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
        let _guard = self.transition_lock.lock().unwrap();

        let chunk_start: Address = start.align_down(BYTES_IN_CHUNK);
        let chunk_end = (start + pages * BYTES_IN_PAGE).align_up(BYTES_IN_CHUNK);
        let aligned_bytes = chunk_end - chunk_start;
        self.storage.bulk_transition_state(
            chunk_start,
            aligned_bytes,
            |group_start, group_bytes, state| match state {
                MapState::Unmapped => {
                    dzmmap_noreplace(group_start, group_bytes, strategy, anno)?;
                    Ok(Some(MapState::Mapped))
                }
                MapState::Protected => {
                    munprotect(group_start, group_bytes, strategy.prot)?;
                    Ok(Some(MapState::Mapped))
                }
                MapState::Quarantined => {
                    unsafe { dzmmap(group_start, group_bytes, strategy, anno) }?;
                    Ok(Some(MapState::Mapped))
                }
                MapState::Mapped => Ok(None),
            },
        )
    }

    /// Is the page pointed to by this address mapped? Returns true if
    /// the page at the given address is mapped.
    ///
    /// Arguments:
    /// * `addr`: Address in question
    fn is_mapped_address(&self, addr: Address) -> bool {
        self.storage.get_state(addr) == Some(MapState::Mapped)
    }

    /// Mark a number of pages as inaccessible.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be protected
    /// * `pages`: Number of pages to be protected
    fn protect(&self, start: Address, pages: usize) {
        let _guard = self.transition_lock.lock().unwrap();

        let chunk_start: Address = start.align_down(BYTES_IN_CHUNK);
        let chunk_end = (start + pages * BYTES_IN_PAGE).align_up(BYTES_IN_CHUNK);
        let aligned_bytes = chunk_end - chunk_start;
        self.storage
            .bulk_transition_state(
                chunk_start,
                aligned_bytes,
                |group_start, group_bytes, state| {
                    let group_end = group_start + group_bytes;

                    match state {
                        MapState::Mapped => {
                            crate::util::memory::mprotect(group_start, group_bytes).unwrap();
                            Ok(Some(MapState::Protected))
                        }
                        MapState::Protected => Ok(None),
                        _ => panic!(
                            "Cannot transition {}-{} to protected",
                            group_start, group_end
                        ),
                    }
                },
            )
            .unwrap();
    }
}

/// The mmap state of a mmap chunk.
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq, Debug, NoUninit)]
enum MapState {
    /// The chunk is unmapped and not managed by MMTk.
    Unmapped,
    /// The chunk is reserved for future use. MMTk reserved the address range but hasn't used it yet.
    /// We have reserved the addresss range with mmap_noreserve with PROT_NONE.
    Quarantined,
    /// The chunk is mapped by MMTk and is in use.
    Mapped,
    /// The chunk is mapped and is also protected by MMTk.
    Protected,
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

    fn get_chunk_map_state(mmapper: &ChunkStateMmapper, chunk: Address) -> Option<MapState> {
        assert_eq!(conversions::mmap_chunk_align_up(chunk), chunk);
        mmapper.get_state(chunk)
    }

    #[test]
    fn ensure_mapped_1page() {
        serial_test(|| {
            let pages = 1;
            with_cleanup(
                || {
                    let mmapper = ChunkStateMmapper::new();
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
                    let mmapper = ChunkStateMmapper::new();
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
                    let mmapper = ChunkStateMmapper::new();
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
                    let mmapper = ChunkStateMmapper::new();
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
                    let mmapper = ChunkStateMmapper::new();
                    let pages_per_chunk = MMAP_CHUNK_BYTES >> LOG_BYTES_IN_PAGE as usize;
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
