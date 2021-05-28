use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::memory::*;
use crate::util::Address;
use atomic::{Atomic, Ordering};
use std::io::Result;

pub trait Mmapper {
    /****************************************************************************
     * Generic mmap and protection functionality
     */

    /**
     * Given an address array describing the regions of virtual memory to be used
     * by MMTk, demand zero map all of them if they are not already mapped.
     *
     * @param spaceMap An address array containing a pairs of start and end
     * addresses for each of the regions to be mappe3d
     */
    fn eagerly_mmap_all_spaces(&self, space_map: &[Address]);

    /**
     * Mark a number of pages as mapped, without making any
     * request to the operating system.  Used to mark pages
     * that the VM has already mapped.
     * @param start Address of the first page to be mapped
     * @param bytes Number of bytes to ensure mapped
     */
    fn mark_as_mapped(&self, start: Address, bytes: usize);

    fn quarantine_address_range(&self, start: Address, pages: usize) -> Result<()>;

    /**
     * Ensure that a range of pages is mmapped (or equivalent).  If the
     * pages are not yet mapped, demand-zero map them. Note that mapping
     * occurs at chunk granularity, not page granularity.<p>
     *
     * NOTE: There is a monotonicity assumption so that only updates require lock
     * acquisition.
     * TODO: Fix the above to support unmapping.
     *
     * @param start The start of the range to be mapped.
     * @param pages The size of the range to be mapped, in pages
     */
    fn ensure_mapped(&self, start: Address, pages: usize) -> Result<()>;

    /**
     * Is the page pointed to by this address mapped ?
     * @param addr Address in question
     * @return {@code true} if the page at the given address is mapped.
     */
    fn is_mapped_address(&self, addr: Address) -> bool;

    /**
     * Mark a number of pages as inaccessible.
     * @param start Address of the first page to be protected
     * @param pages Number of pages to be protected
     */
    fn protect(&self, start: Address, pages: usize);
}

/// The mmap state of a mmap chunk.
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) enum MapState {
    Unmapped,
    Quarantined,
    Mapped,
    Protected,
}

impl MapState {
    /// Check the current MapState of the chunk, and transition the chunk to MapState::Mapped.
    /// The caller should hold a lock before invoking this method.
    pub(super) fn transition_to_mapped(
        state: &Atomic<MapState>,
        mmap_start: Address,
        // metadata: &SideMetadata,
    ) -> Result<()> {
        let res = match state.load(Ordering::Relaxed) {
            MapState::Unmapped => {
                // map data
                // dzmmap_noreplace(mmap_start, MMAP_CHUNK_BYTES)
                //     .and(metadata.try_map_metadata_space(mmap_start, MMAP_CHUNK_BYTES))
                dzmmap_noreplace(mmap_start, MMAP_CHUNK_BYTES)
            }
            MapState::Protected => munprotect(mmap_start, MMAP_CHUNK_BYTES),
            MapState::Quarantined => unsafe { dzmmap(mmap_start, MMAP_CHUNK_BYTES) },
            // might have become MapState::Mapped here
            MapState::Mapped => Ok(()),
        };
        if res.is_ok() {
            state.store(MapState::Mapped, Ordering::Relaxed);
        }
        res
    }

    pub(super) fn transition_to_quarantined(
        state: &Atomic<MapState>,
        mmap_start: Address,
    ) -> Result<()> {
        let res = match state.load(Ordering::Relaxed) {
            MapState::Unmapped => {
                // map data
                // dzmmap_noreplace(mmap_start, MMAP_CHUNK_BYTES)
                //     .and(metadata.try_map_metadata_space(mmap_start, MMAP_CHUNK_BYTES))
                mmap_noreserve(mmap_start, MMAP_CHUNK_BYTES)
            }
            MapState::Quarantined => Ok(()),
            MapState::Mapped => panic!("Cannot quarantine mapped memory"),
            MapState::Protected => panic!("Cannot quarantine protected memory"),
        };
        if res.is_ok() {
            state.store(MapState::Quarantined, Ordering::Relaxed);
        }
        res
    }

    /// Check the current MapState of the chunk, and transition the chunk to MapState::Protected.
    /// The caller should hold a lock before invoking this method.
    pub(super) fn transition_to_protected(
        state: &Atomic<MapState>,
        mmap_start: Address,
    ) -> Result<()> {
        match state.load(Ordering::Relaxed) {
            MapState::Mapped => {
                crate::util::memory::mprotect(mmap_start, MMAP_CHUNK_BYTES).unwrap();
                state.store(MapState::Protected, Ordering::Relaxed);
            }
            MapState::Protected => {}
            _ => panic!("Cannot transition {:?} to protected", mmap_start),
        }
        Ok(())
    }
}
