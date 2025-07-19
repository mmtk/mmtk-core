use super::MapState;
use crate::util::conversions::raw_is_aligned;
use crate::util::heap::layout::mmapper::MapStateStorage;
use crate::util::rust_util::rev_group::RevisitableGroupByForIterator;
use crate::util::Address;

use crate::util::constants::*;
use crate::util::heap::layout::vm_layout::*;
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
    fn get_state(&self, chunk: Address) -> Option<MapState> {
        let index = chunk >> LOG_BYTES_IN_CHUNK;
        let slot = self.mapped.get(index)?;
        Some(slot.load(Ordering::SeqCst))
    }

    // fn set_state(&self, chunk: Address, state: MapState) {
    //     let index = chunk >> LOG_BYTES_IN_CHUNK;
    //     let Some(slot) = self.mapped.get(index) else {
    //         panic!("Chunk {chunk} out of range.");
    //     };
    //     slot.store(state, Ordering::SeqCst);
    // }

    fn bulk_set_state(&self, start: Address, bytes: usize, state: MapState) {
        debug_assert!(
            start.is_aligned_to(BYTES_IN_CHUNK),
            "start {start} is not aligned"
        );
        debug_assert!(
            raw_is_aligned(bytes, BYTES_IN_CHUNK),
            "bytes {bytes} is not aligned"
        );
        let index_start = start >> LOG_BYTES_IN_CHUNK;
        let index_limit = (start + bytes) >> LOG_BYTES_IN_CHUNK;
        if index_start >= self.mapped.len() {
            panic!("chunk {start} out of range");
        }
        if index_limit >= self.mapped.len() {
            panic!("bytes {bytes} out of range");
        }
        for index in index_start..index_limit {
            self.mapped[index].store(state, Ordering::Relaxed);
        }
    }

    fn bulk_transition_state<F>(&self, start: Address, bytes: usize, mut transformer: F) -> Result<()>
    where
        F: FnMut(Address, usize, MapState) -> Result<Option<MapState>>,
    {
        debug_assert!(
            start.is_aligned_to(BYTES_IN_CHUNK),
            "start {start} is not aligned"
        );
        debug_assert!(
            raw_is_aligned(bytes, BYTES_IN_CHUNK),
            "bytes {bytes} is not aligned"
        );
        let index_start = start >> LOG_BYTES_IN_CHUNK;
        let index_limit = (start + bytes) >> LOG_BYTES_IN_CHUNK;
        if index_start >= self.mapped.len() {
            panic!("start {start} out of range");
        }
        if index_limit >= self.mapped.len() {
            panic!("bytes {bytes} out of range");
        }

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
            if let Some(new_state) = transformer(group_start_addr, group_bytes, state)? {
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
