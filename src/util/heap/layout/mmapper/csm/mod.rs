use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::util::conversions::raw_is_aligned;
use crate::util::heap::layout::vm_layout::*;
use crate::util::heap::layout::Mmapper;
use crate::util::memory::*;
use crate::util::Address;
use bytemuck::NoUninit;
use std::{io::Result, sync::Mutex};

mod byte_map_storage;
#[cfg(target_pointer_width = "64")]
mod two_level_storage;

#[cfg(target_pointer_width = "32")]
type ChosenMapStateStorage = byte_map_storage::ByteMapStateStorage;
#[cfg(target_pointer_width = "64")]
type ChosenMapStateStorage = two_level_storage::TwoLevelStateStorage;

/// A range of whole chunks.  Always aligned.
///
/// This type is used internally by the chunk state mmapper and its storage backends.
#[derive(Clone, Copy)]
struct ChunkRange {
    start: Address,
    bytes: usize,
}

impl ChunkRange {
    fn new_aligned(start: Address, bytes: usize) -> Self {
        debug_assert!(
            start.is_aligned_to(BYTES_IN_CHUNK),
            "start {start} is not chunk-aligned"
        );
        debug_assert!(
            raw_is_aligned(bytes, BYTES_IN_CHUNK),
            "bytes 0x{bytes:x} is not a multiple of chunks"
        );
        Self { start, bytes }
    }

    fn new_unaligned(start: Address, bytes: usize) -> Self {
        let start_aligned = start.align_down(BYTES_IN_CHUNK);
        let end_aligned = (start + bytes).align_up(BYTES_IN_CHUNK);
        Self::new_aligned(start_aligned, end_aligned - start_aligned)
    }

    fn limit(&self) -> Address {
        self.start + self.bytes
    }

    fn is_within_limit(&self, limit: Address) -> bool {
        self.limit() <= limit
    }
}

impl std::fmt::Display for ChunkRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{}", self.start, self.limit())
    }
}

/// The back-end storage of [`ChunkStateMmapper`].  It is responsible for holding the states of each
/// chunk (eagerly or lazily) and transitioning the states in bulk.
trait MapStateStorage {
    /// Return the state of a given `chunk` (must be aligned).
    ///
    /// Note that all chunks are logically `MapState::Unmapped` before the states are stored.  They
    /// include chunks outside the mappable address range.
    fn get_state(&self, chunk: Address) -> MapState;

    /// Set all chunks within `range` to `state`.
    fn bulk_set_state(&self, range: ChunkRange, state: MapState);

    /// Visit the chunk states within `range` and allow the `transformer` callback to inspect and
    /// change the states.
    ///
    /// It visits chunks from low to high addresses, and calls `transformer(group_range,
    /// group_state)` for each contiguous chunk range `group_range` that have the same state
    /// `group_state`.  `transformer` can take actions accordingly and return one of the three
    /// values:
    /// -   `Err(err)`: Stop visiting and return `Err(err)` from `bulk_transition_state`
    ///     immediately.
    /// -   `Ok(None)`: Continue visiting the next chunk range without changing chunk states.
    /// -   `Ok(Some(new_state))`: Set the state of all chunks within `group_range` to `new_state`.
    ///
    /// Return `Ok(())` if finished visiting all chunks normally.
    fn bulk_transition_state<F>(&self, range: ChunkRange, transformer: F) -> Result<()>
    where
        F: FnMut(ChunkRange, MapState) -> Result<Option<MapState>>;
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
    fn get_state(&self, chunk: Address) -> MapState {
        self.storage.get_state(chunk)
    }
}

/// Generic mmap and protection functionality
impl Mmapper for ChunkStateMmapper {
    fn log_granularity(&self) -> u8 {
        LOG_BYTES_IN_CHUNK as u8
    }

    fn eagerly_mmap_all_spaces(&self, _space_map: &[Address]) {
        unimplemented!()
    }

    fn mark_as_mapped(&self, start: Address, bytes: usize) {
        let _guard = self.transition_lock.lock().unwrap();

        let range = ChunkRange::new_unaligned(start, bytes);
        self.storage.bulk_set_state(range, MapState::Mapped);
    }

    fn quarantine_address_range(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
        let _guard = self.transition_lock.lock().unwrap();

        let bytes = pages << LOG_BYTES_IN_PAGE;
        let range = ChunkRange::new_unaligned(start, bytes);

        self.storage
            .bulk_transition_state(range, |group_range, state| {
                let group_start: Address = group_range.start;
                let group_bytes = group_range.bytes;

                match state {
                    MapState::Unmapped => {
                        trace!("Trying to quarantine {group_range}");
                        mmap_noreserve(group_start, group_bytes, strategy, anno)?;
                        Ok(Some(MapState::Quarantined))
                    }
                    MapState::Quarantined => {
                        trace!("Already quarantine {group_range}");
                        Ok(None)
                    }
                    MapState::Mapped => {
                        trace!("Already mapped {group_range}");
                        Ok(None)
                    }
                    MapState::Protected => {
                        panic!("Cannot quarantine protected memory")
                    }
                }
            })
    }

    fn ensure_mapped(
        &self,
        start: Address,
        pages: usize,
        strategy: MmapStrategy,
        anno: &MmapAnnotation,
    ) -> Result<()> {
        let _guard = self.transition_lock.lock().unwrap();

        let bytes = pages << LOG_BYTES_IN_PAGE;
        let range = ChunkRange::new_unaligned(start, bytes);

        self.storage
            .bulk_transition_state(range, |group_range, state| {
                let group_start: Address = group_range.start;
                let group_bytes = group_range.bytes;

                match state {
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
                }
            })
    }

    fn is_mapped_address(&self, addr: Address) -> bool {
        self.storage.get_state(addr) == MapState::Mapped
    }

    fn protect(&self, start: Address, pages: usize) {
        let _guard = self.transition_lock.lock().unwrap();

        let bytes = pages << LOG_BYTES_IN_PAGE;
        let range = ChunkRange::new_unaligned(start, bytes);

        self.storage
            .bulk_transition_state(range, |group_range, state| {
                let group_start: Address = group_range.start;
                let group_bytes = group_range.bytes;

                match state {
                    MapState::Mapped => {
                        crate::util::memory::mprotect(group_start, group_bytes).unwrap();
                        Ok(Some(MapState::Protected))
                    }
                    MapState::Protected => Ok(None),
                    _ => panic!("Cannot transition {group_range} to protected",),
                }
            })
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
    use crate::util::memory;
    use crate::util::test_util::CHUNK_STATE_MMAPPER_TEST_REGION;
    use crate::util::test_util::{serial_test, with_cleanup};
    use crate::util::{conversions, Address};

    const FIXED_ADDRESS: Address = CHUNK_STATE_MMAPPER_TEST_REGION.start;
    const MAX_BYTES: usize = CHUNK_STATE_MMAPPER_TEST_REGION.size;

    fn pages_to_chunks_up(pages: usize) -> usize {
        conversions::raw_align_up(pages, BYTES_IN_CHUNK) / BYTES_IN_CHUNK
    }

    fn get_chunk_map_state(mmapper: &ChunkStateMmapper, chunk: Address) -> MapState {
        chunk.is_aligned_to(BYTES_IN_CHUNK);
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
                            MapState::Mapped
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
            let pages = BYTES_IN_CHUNK >> LOG_BYTES_IN_PAGE as usize;
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
                            MapState::Mapped
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
            let pages = (BYTES_IN_CHUNK + BYTES_IN_CHUNK / 2) >> LOG_BYTES_IN_PAGE as usize;
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
                            MapState::Mapped
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
                    let pages_per_chunk = BYTES_IN_CHUNK >> LOG_BYTES_IN_PAGE as usize;
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
                        MapState::Protected
                    );
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS + BYTES_IN_CHUNK),
                        MapState::Mapped
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
                    let pages_per_chunk = BYTES_IN_CHUNK >> LOG_BYTES_IN_PAGE as usize;
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
                        MapState::Mapped
                    );
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS + BYTES_IN_CHUNK),
                        MapState::Mapped
                    );

                    // protect 1 chunk
                    mmapper.protect(FIXED_ADDRESS, pages_per_chunk);

                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS),
                        MapState::Protected
                    );
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS + BYTES_IN_CHUNK),
                        MapState::Mapped
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
                        MapState::Mapped
                    );
                    assert_eq!(
                        get_chunk_map_state(&mmapper, FIXED_ADDRESS + BYTES_IN_CHUNK),
                        MapState::Mapped
                    );
                },
                || {
                    memory::munmap(FIXED_ADDRESS, MAX_BYTES).unwrap();
                },
            )
        })
    }
}
