use super::MapState;
use crate::util::heap::layout::mmapper::csm::ChunkRange;
use crate::util::heap::layout::mmapper::csm::MapStateStorage;
use crate::util::rust_util::rev_group::RevisitableGroupByForIterator;
use crate::util::Address;

use crate::util::heap::layout::vm_layout::*;
use std::fmt;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use atomic::Atomic;
use std::io::Result;

/// For now, we only use `ByteMapStateStorage` for 32-bit address range.
const MMAP_NUM_CHUNKS: usize = 1 << (32 - LOG_BYTES_IN_CHUNK);

/// A [`MapStateStorage`] implementation based on a simple array.
///
/// Currently it is sized to cover a 32-bit address range.
pub struct ByteMapStateStorage {
    lock: Mutex<()>,
    mapped: [Atomic<MapState>; MMAP_NUM_CHUNKS],
}

impl fmt::Debug for ByteMapStateStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ByteMapStateStorage({})", MMAP_NUM_CHUNKS)
    }
}

impl MapStateStorage for ByteMapStateStorage {
    fn get_state(&self, chunk: Address) -> MapState {
        let index = chunk >> LOG_BYTES_IN_CHUNK;
        let Some(slot) = self.mapped.get(index) else {
            return MapState::Unmapped;
        };
        slot.load(Ordering::SeqCst)
    }

    fn bulk_set_state(&self, range: ChunkRange, state: MapState) {
        let index_start = range.start >> LOG_BYTES_IN_CHUNK;
        let index_limit = range.limit() >> LOG_BYTES_IN_CHUNK;
        for index in index_start..index_limit {
            self.mapped[index].store(state, Ordering::Relaxed);
        }
    }

    fn bulk_transition_state<F>(&self, range: ChunkRange, mut transformer: F) -> Result<()>
    where
        F: FnMut(ChunkRange, MapState) -> Result<Option<MapState>>,
    {
        let index_start = range.start >> LOG_BYTES_IN_CHUNK;
        let index_limit = range.limit() >> LOG_BYTES_IN_CHUNK;

        let mut group_start = index_start;
        for group in self.mapped.as_slice()[index_start..index_limit]
            .iter()
            .revisitable_group_by(|s| s.load(Ordering::SeqCst))
        {
            let state = group.key;
            let group_end = group_start + group.len;
            let group_start_addr =
                unsafe { Address::from_usize(group_start << LOG_BYTES_IN_CHUNK) };
            let group_bytes = group.len << LOG_BYTES_IN_CHUNK;
            let group_range = ChunkRange::new_aligned(group_start_addr, group_bytes);
            if let Some(new_state) = transformer(group_range, state)? {
                for index in group_start..group_end {
                    self.mapped[index].store(new_state, Ordering::Relaxed);
                }
            }
            group_start = group_end;
        }

        Ok(())
    }
}

impl ByteMapStateStorage {
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

        ByteMapStateStorage {
            lock: Mutex::new(()),
            mapped: [INITIAL_ENTRY; MMAP_NUM_CHUNKS],
        }
    }
}

impl Default for ByteMapStateStorage {
    fn default() -> Self {
        Self::new()
    }
}
