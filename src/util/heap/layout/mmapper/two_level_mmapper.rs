//! This module contains [`TwoLevelMmapper`], an implementation of [`Mmapper`] that is designed to
//! work well on 64-bit machines.  Currently it supports 48-bit address spaces, and many constants
//! and data structures (such as [`Slab`]) are larger than `i32::MAX`.  For this reason, this module
//! is only available on 64-bit machines.

use super::MapState;
use crate::util::conversions::raw_is_aligned;
use crate::util::heap::layout::mmapper::MapStateStorage;
use crate::util::heap::layout::vm_layout::*;
use crate::util::rust_util::atomic_box::OnceOptionBox;
use crate::util::rust_util::rev_group::RevisitableGroupByForIterator;
use crate::util::Address;
use atomic::{Atomic, Ordering};
use std::fmt;
use std::io::Result;
use std::sync::Mutex;

/// Logarithm of the address space size a user-space program is allowed to use.
/// This is enough for ARM64, x86_64 and some other architectures.
/// Feel free to increase it if we plan to support larger address spaces.
const LOG_MAPPABLE_BYTES: usize = 48;
/// Address space size a user-space program is allowed to use.
const MAPPABLE_BYTES: usize = 1 << LOG_MAPPABLE_BYTES;

/// Log number of bytes per slab.
/// For a two-level array, it is advisable to choose the arithmetic mean of [`LOG_MAPPABLE_BYTES`]
/// and [`LOG_MMAP_CHUNK_BYTES`] in order to make [`MMAP_SLAB_BYTES`] the geometric mean of
/// [`MAPPABLE_BYTES`] and [`MMAP_CHUNK_BYTES`].  This will balance the array size of
/// [`TwoLevelMmapper::slabs`] and [`Slab`].
///
/// TODO: Use `usize::midpoint` after bumping MSRV to 1.85
const LOG_MMAP_SLAB_BYTES: usize =
    LOG_MMAP_CHUNK_BYTES + (LOG_MAPPABLE_BYTES - LOG_MMAP_CHUNK_BYTES) / 2;
/// Number of bytes per slab.
const MMAP_SLAB_BYTES: usize = 1 << LOG_MMAP_SLAB_BYTES;

/// Log number of chunks per slab.
const LOG_MMAP_CHUNKS_PER_SLAB: usize = LOG_MMAP_SLAB_BYTES - LOG_MMAP_CHUNK_BYTES;
/// Number of chunks per slab.
const MMAP_CHUNKS_PER_SLAB: usize = 1 << LOG_MMAP_CHUNKS_PER_SLAB;

/// Mask for getting in-slab bits from an address.
/// Invert this to get out-of-slab bits.
const MMAP_SLAB_MASK: usize = (1 << LOG_MMAP_SLAB_BYTES) - 1;

/// Logarithm of maximum number of slabs.
const LOG_MAX_SLABS: usize = LOG_MAPPABLE_BYTES - LOG_MMAP_SLAB_BYTES;
/// maximum number of slabs.
const MAX_SLABS: usize = 1 << LOG_MAX_SLABS;

/// The slab type.  Each slab holds the `MapState` of multiple chunks.
type Slab = [Atomic<MapState>; MMAP_CHUNKS_PER_SLAB];

/// A two-level implementation of `Mmapper`.
///
/// It is essentially a lazily initialized array of [`Atomic<MapState>`].  Because it is designed to
/// govern a large address range, and the array is sparse, we use a two-level design.  The higher
/// level holds a vector of slabs, and each slab holds an array of [`Atomic<MapState>`].  Each slab
/// governs an aligned region of [`MMAP_CHUNKS_PER_SLAB`] chunks.  Slabs are lazily created when the
/// user intends to write into one of its `MapState`.
pub struct TwoLevelMmapper {
    transition_lock: Mutex<()>,
    /// Slabs
    slabs: Vec<OnceOptionBox<Slab>>,
}

unsafe impl Send for TwoLevelMmapper {}
unsafe impl Sync for TwoLevelMmapper {}

impl fmt::Debug for TwoLevelMmapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TwoLevelMapper({})", 1 << LOG_MAX_SLABS)
    }
}

impl MapStateStorage for TwoLevelMmapper {
    fn get_state(&self, chunk: Address) -> Option<MapState> {
        self.slab_table(chunk)
            .map(|slab| slab[Self::in_slab_index(chunk)].load(Ordering::Relaxed))
    }

    fn bulk_set_state(&self, start: Address, bytes: usize, state: MapState) {
        self.foreach_slab_slice_for_write(start, bytes, |_low, _high, slice| {
            for slot in slice {
                slot.store(state, Ordering::Relaxed);
            }
        });
    }

    fn bulk_transition_state<F>(
        &self,
        start: Address,
        bytes: usize,
        mut transformer: F,
    ) -> Result<()>
    where
        F: FnMut(Address, usize, MapState) -> Result<Option<MapState>>,
    {
        let mut slice_indices = Vec::new();

        self.foreach_slab_slice_for_write(start, bytes, |_low, _high, slice| {
            slice_indices.push(slice);
        });

        // Chunk index from `start`.
        let mut start_index: usize = 0usize;

        for group in slice_indices
            .iter()
            .copied()
            .flatten()
            .revisitable_group_by(|s| s.load(Ordering::Relaxed))
        {
            let state = group.key;
            let end_index = start_index + group.len;
            let start_addr = start + MMAP_CHUNK_BYTES * start_index;
            let group_bytes = group.len << LOG_MMAP_CHUNK_BYTES;

            if let Some(new_state) = transformer(start_addr, group_bytes, state)? {
                for slot in group {
                    slot.store(new_state, Ordering::Relaxed);
                }
            }

            start_index = end_index;
        }

        Ok(())
    }
}

impl TwoLevelMmapper {
    pub fn new() -> Self {
        Self {
            transition_lock: Default::default(),
            slabs: unsafe { crate::util::rust_util::zeroed_alloc::new_zeroed_vec(MAX_SLABS) },
        }
    }

    fn new_slab() -> Slab {
        std::array::from_fn(|_| Atomic::new(MapState::Unmapped))
    }

    fn slab_table(&self, addr: Address) -> Option<&Slab> {
        let index: usize = Self::slab_index(addr);
        let slot = self.slabs.get(index)?;
        // Note: We don't need acquire here.  See `get_or_allocate_slab_table`.
        slot.get(Ordering::Relaxed)
    }

    fn get_or_allocate_slab_table(&self, addr: Address) -> &Slab {
        let index: usize = Self::slab_index(addr);
        let Some(slot) = self.slabs.get(index) else {
            panic!("Cannot allocate slab for address: {addr}");
        };
        // Note: We set both order_load and order_store to `Relaxed` because we never populate the
        // content of the slab before making the `OnceOptionBox` point to the new slab. For this
        // reason, the release-acquire relation is not needed here.
        slot.get_or_init(Ordering::Relaxed, Ordering::Relaxed, Self::new_slab)
    }

    fn slab_index(addr: Address) -> usize {
        addr >> LOG_MMAP_SLAB_BYTES
    }

    fn in_slab_index(addr: Address) -> usize {
        (addr & MMAP_SLAB_MASK) >> LOG_BYTES_IN_CHUNK
    }

    fn foreach_slab_slice_for_write<'s, F>(&'s self, start: Address, bytes: usize, mut f: F)
    where
        F: FnMut(Address, Address, &'s [Atomic<MapState>]),
    {
        debug_assert!(
            start.is_aligned_to(BYTES_IN_CHUNK),
            "start {start} is not aligned"
        );
        debug_assert!(
            raw_is_aligned(bytes, BYTES_IN_CHUNK),
            "bytes {bytes} is not aligned"
        );
        if start.as_usize() >= MAPPABLE_BYTES {
            panic!("start {start} out of range");
        }
        let limit = start + bytes;
        if limit.as_usize() >= MAPPABLE_BYTES {
            panic!("bytes {bytes} out of range");
        }

        let mut low = start;
        while low < limit {
            let high = (low + MMAP_SLAB_BYTES)
                .align_down(MMAP_SLAB_BYTES)
                .min(limit);

            let slab = self.get_or_allocate_slab_table(low);
            let low_index = Self::in_slab_index(low);
            let high_index = Self::in_slab_index(high);
            let ub_index = if high_index == 0 {
                MMAP_CHUNKS_PER_SLAB
            } else {
                high_index
            };
            f(low, high, &slab[low_index..ub_index]);

            low = high;
        }
    }

    fn chunk_index_to_address(base: Address, chunk: usize) -> Address {
        base + (chunk << LOG_MMAP_CHUNK_BYTES)
    }

    /// Align `addr` down to slab size.
    fn slab_align_down(addr: Address) -> Address {
        addr.align_down(MMAP_SLAB_BYTES)
    }

    /// Get the base address of the next slab after the slab that contains `addr`.
    fn slab_limit(addr: Address) -> Address {
        Self::slab_align_down(addr) + MMAP_SLAB_BYTES
    }

    /// Return the index of the chunk that contains `addr` within the slab starting at `slab`.
    /// If `addr` is beyond the end of the slab, the result could be beyond the end of the slab.
    fn chunk_index(slab: Address, addr: Address) -> usize {
        let delta = addr - slab;
        delta >> LOG_MMAP_CHUNK_BYTES
    }
}

impl Default for TwoLevelMmapper {
    fn default() -> Self {
        Self::new()
    }
}
