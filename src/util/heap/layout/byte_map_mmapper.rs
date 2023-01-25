use super::mmapper::MapState;
use super::Mmapper;
use crate::util::Address;

use crate::util::constants::*;
use crate::util::conversions::pages_to_bytes;
use crate::util::heap::layout::vm_layout_constants::*;
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
            MapState::transition_to_mapped(&self.mapped[chunk], mmap_start).unwrap();
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
            MapState::transition_to_quarantined(&self.mapped[chunk], mmap_start).unwrap();
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
