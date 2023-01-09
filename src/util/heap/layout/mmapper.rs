use crate::util::heap::layout::vm_layout_constants::*;
use crate::util::memory::*;
use crate::util::Address;
use atomic::{Atomic, Ordering};
use std::io::Result;

/// Generic mmap and protection functionality
pub trait Mmapper: Sync {
    /// Given an address array describing the regions of virtual memory to be used
    /// by MMTk, demand zero map all of them if they are not already mapped.
    ///
    /// Arguments:
    /// * `spaceMap`: An address array containing a pairs of start and end
    ///   addresses for each of the regions to be mapped
    fn eagerly_mmap_all_spaces(&self, space_map: &[Address]);

    /// Mark a number of pages as mapped, without making any
    /// request to the operating system.  Used to mark pages
    /// that the VM has already mapped.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be mapped
    /// * `bytes`: Number of bytes to ensure mapped
    fn mark_as_mapped(&self, start: Address, bytes: usize);

    /// Quarantine/reserve address range. We mmap from the OS with no reserve and with PROT_NONE,
    /// which should be little overhead. This ensures that we can reserve certain address range that
    /// we can use if needed. Quarantined memory needs to be mapped before it can be used.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be quarantined
    /// * `bytes`: Number of bytes to quarantine from the start
    fn quarantine_address_range(&self, start: Address, pages: usize) -> Result<()>;

    /// Ensure that a range of pages is mmapped (or equivalent).  If the
    /// pages are not yet mapped, demand-zero map them. Note that mapping
    /// occurs at chunk granularity, not page granularity.<p>
    ///
    /// Argumetns:
    /// `start`: The start of the range to be mapped.
    /// `pages`: The size of the range to be mapped, in pages
    // NOTE: There is a monotonicity assumption so that only updates require lock
    // acquisition.
    // TODO: Fix the above to support unmapping.
    fn ensure_mapped(&self, start: Address, pages: usize) -> Result<()>;

    /// Is the page pointed to by this address mapped? Returns true if
    /// the page at the given address is mapped.
    ///
    /// Arguments:
    /// * `addr`: Address in question
    fn is_mapped_address(&self, addr: Address) -> bool;

    /// Mark a number of pages as inaccessible.
    ///
    /// Arguments:
    /// * `start`: Address of the first page to be protected
    /// * `pages`: Number of pages to be protected
    fn protect(&self, start: Address, pages: usize);
}

/// The mmap state of a mmap chunk.
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(super) enum MapState {
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

impl MapState {
    /// Check the current MapState of the chunk, and transition the chunk to MapState::Mapped.
    /// The caller should hold a lock before invoking this method.
    pub(super) fn transition_to_mapped(
        state: &Atomic<MapState>,
        mmap_start: Address,
    ) -> Result<()> {
        trace!(
            "Trying to map {} - {}",
            mmap_start,
            mmap_start + MMAP_CHUNK_BYTES
        );
        let res = match state.load(Ordering::Relaxed) {
            MapState::Unmapped => dzmmap_noreplace(mmap_start, MMAP_CHUNK_BYTES),
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

    /// Check the current MapState of the chunk, and transition the chunk to MapState::Quarantined.
    /// The caller should hold a lock before invoking this method.
    pub(super) fn transition_to_quarantined(
        state: &Atomic<MapState>,
        mmap_start: Address,
    ) -> Result<()> {
        trace!(
            "Trying to quarantine {} - {}",
            mmap_start,
            mmap_start + MMAP_CHUNK_BYTES
        );
        let res = match state.load(Ordering::Relaxed) {
            MapState::Unmapped => mmap_noreserve(mmap_start, MMAP_CHUNK_BYTES),
            MapState::Quarantined => Ok(()),
            MapState::Mapped => {
                // If a chunk is mapped by us and we try to quanrantine it, we simply don't do anything.
                // We allow this as it is possible to have a situation like this:
                // we have global side metdata S, and space A and B. We quanrantine memory X for S for A, then map
                // X for A, and then we quanrantine memory Y for S for B. It is possible that X and Y is the same chunk,
                // so the chunk is already mapped for A, and we try quanrantine it for B. We simply allow this transition.
                return Ok(());
            }
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
