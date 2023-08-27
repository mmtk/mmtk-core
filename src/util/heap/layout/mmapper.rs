use crate::util::heap::layout::vm_layout::*;
use crate::util::memory::*;
use crate::util::rust_util::rev_group::RevisitableGroupByForIterator;
use crate::util::Address;
use atomic::{Atomic, Ordering};
use std::io::Result;

/// Generic mmap and protection functionality
pub trait Mmapper: Sync {
    /// Set mmap strategy
    fn set_mmap_strategy(&self, strategy: MmapStrategy);

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
    /// occurs at chunk granularity, not page granularity.
    ///
    /// Arguments:
    /// * `start`: The start of the range to be mapped.
    /// * `pages`: The size of the range to be mapped, in pages
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
        strategy: MmapStrategy,
    ) -> Result<()> {
        trace!(
            "Trying to map {} - {}",
            mmap_start,
            mmap_start + MMAP_CHUNK_BYTES
        );
        let res = match state.load(Ordering::Relaxed) {
            MapState::Unmapped => dzmmap_noreplace(mmap_start, MMAP_CHUNK_BYTES, strategy),
            MapState::Protected => munprotect(mmap_start, MMAP_CHUNK_BYTES),
            MapState::Quarantined => unsafe { dzmmap(mmap_start, MMAP_CHUNK_BYTES, strategy) },
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
        strategy: MmapStrategy,
    ) -> Result<()> {
        trace!(
            "Trying to quarantine {} - {}",
            mmap_start,
            mmap_start + MMAP_CHUNK_BYTES
        );
        let res = match state.load(Ordering::Relaxed) {
            MapState::Unmapped => mmap_noreserve(mmap_start, MMAP_CHUNK_BYTES, strategy),
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

    /// Equivalent to calling `transition_to_quarantined` on each element of `states`, but faster.
    /// The caller should hold a lock before invoking this method.
    ///
    /// The memory region to transition starts from `mmap_start`. The size is the chunk size
    /// multiplied by the total number of `Atomic<MapState>` in the nested slice of slices.
    ///
    /// This function is introduced to speed up initialization of MMTk.  MMTk tries to quarantine
    /// very large amount of memory during start-up.  If we quarantine one chunk at a time, it will
    /// require thousands of `mmap` calls to cover gigabytes of quarantined memory, introducing a
    /// noticeable delay.
    ///
    /// Arguments:
    ///
    /// * `state_slices`: A slice of slices. Each inner slice is a part of a `Slab`.
    /// * `mmap_start`: The start of the region to transition.
    pub(super) fn bulk_transition_to_quarantined(
        state_slices: &[&[Atomic<MapState>]],
        mmap_start: Address,
        strategy: MmapStrategy,
    ) -> Result<()> {
        trace!(
            "Trying to bulk-quarantine {} - {}",
            mmap_start,
            mmap_start + MMAP_CHUNK_BYTES * state_slices.iter().map(|s| s.len()).sum::<usize>(),
        );

        let mut start_index = 0;

        for group in state_slices
            .iter()
            .copied()
            .flatten()
            .revisitable_group_by(|s| s.load(Ordering::Relaxed))
        {
            let end_index = start_index + group.len;
            let start_addr = mmap_start + MMAP_CHUNK_BYTES * start_index;
            let end_addr = mmap_start + MMAP_CHUNK_BYTES * end_index;

            match group.key {
                MapState::Unmapped => {
                    trace!("Trying to quarantine {} - {}", start_addr, end_addr);
                    mmap_noreserve(start_addr, end_addr - start_addr, strategy)?;

                    for state in group {
                        state.store(MapState::Quarantined, Ordering::Relaxed);
                    }
                }
                MapState::Quarantined => {
                    trace!("Already quarantine {} - {}", start_addr, end_addr);
                }
                MapState::Mapped => {
                    trace!("Already mapped {} - {}", start_addr, end_addr);
                }
                MapState::Protected => {
                    panic!("Cannot quarantine protected memory")
                }
            }

            start_index = end_index;
        }

        Ok(())
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
