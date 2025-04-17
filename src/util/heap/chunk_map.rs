use crate::scheduler::GCWork;
use crate::util::linear_scan::Region;
use crate::util::linear_scan::RegionIterator;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::Address;
use crate::vm::VMBinding;
use spin::Mutex;
use std::ops::Range;

/// Data structure to reference a MMTk 4 MB chunk.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Chunk(Address);

impl Region for Chunk {
    const LOG_BYTES: usize = crate::util::heap::layout::vm_layout::LOG_BYTES_IN_CHUNK;

    fn from_aligned_address(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    fn start(&self) -> Address {
        self.0
    }
}

impl Chunk {
    /// Chunk constant with zero address
    // FIXME: We use this as an empty value. What if we actually use the first chunk?
    pub const ZERO: Self = Self(Address::ZERO);

    /// Get an iterator for regions within this chunk.
    pub fn iter_region<R: Region>(&self) -> RegionIterator<R> {
        // R should be smaller than a chunk
        debug_assert!(R::LOG_BYTES < Self::LOG_BYTES);
        // R should be aligned to chunk boundary
        debug_assert!(R::is_aligned(self.start()));
        debug_assert!(R::is_aligned(self.end()));

        let start = R::from_aligned_address(self.start());
        let end = R::from_aligned_address(self.end());
        RegionIterator::<R>::new(start, end)
    }
}

/// The allocation state for a chunk in the chunk map. It includes whether each chunk is allocated or free, and the space the chunk belongs to.
/// Highest bit: 0 = free, 1 = allocated
/// Lower 4 bits: Space index (0-15)
#[repr(transparent)]
#[derive(PartialEq, Clone, Copy)]
pub struct ChunkState(u8);

impl ChunkState {
    /// Create a new ChunkState that represents being allocated in the given space
    pub fn allocated(space_index: usize) -> ChunkState {
        debug_assert!(space_index < crate::util::heap::layout::heap_parameters::MAX_SPACES);
        let mut encode = space_index as u8;
        encode |= 0x80;
        ChunkState(encode)
    }
    /// Create a new ChunkState that represents being free in the given space
    pub fn free(space_index: usize) -> ChunkState {
        debug_assert!(space_index < crate::util::heap::layout::heap_parameters::MAX_SPACES);
        ChunkState(space_index as u8)
    }
    /// Is the chunk free?
    pub fn is_free(&self) -> bool {
        self.0 & 0x80 == 0
    }
    /// Is the chunk allocated?
    pub fn is_allocated(&self) -> bool {
        !self.is_free()
    }
    /// Get the space index of the chunk
    pub fn get_space_index(&self) -> usize {
        debug_assert!(self.is_allocated());
        let index = (self.0 & 0x0F) as usize;
        debug_assert!(index < crate::util::heap::layout::heap_parameters::MAX_SPACES);
        index
    }
}

impl std::fmt::Debug for ChunkState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_free() {
            write!(f, "Free({})", self.get_space_index())
        } else {
            write!(f, "Allocated({})", self.get_space_index())
        }
    }
}

/// A byte-map to record all the allocated chunks.
/// A plan can use this to maintain records for the chunks that they used, and the states of the chunks.
/// Any plan that uses the chunk map should include the `ALLOC_TABLE` spec in their local sidemetadata specs.
///
/// A chunk map is created for a space (identified by the space index), and will only update or list chunks for that space.
pub struct ChunkMap {
    /// The space that uses this chunk map.
    space_index: usize,
    /// The range of chunks that are used by the space. The range only records the lowest chunk and the highest chunk.
    /// All the chunks that are used for the space are within the range, but not necessarily that all the chunks in the range
    /// are used for the space. Spaces may be discontiguous, thus the range may include chunks that do not belong to the space.
    /// We need to use the space index in the chunk map and the space index encoded with the chunk state to know if
    /// the chunk belongs to the current space.
    chunk_range: Mutex<Range<Chunk>>,
}

impl ChunkMap {
    /// Chunk alloc table
    pub const ALLOC_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::CHUNK_MARK;

    pub fn new(space_index: usize) -> Self {
        Self {
            space_index,
            chunk_range: Mutex::new(Chunk::ZERO..Chunk::ZERO),
        }
    }

    /// Set a chunk as allocated, or as free.
    pub fn set_allocated(&self, chunk: Chunk, allocated: bool) {
        let state = if allocated {
            ChunkState::allocated(self.space_index)
        } else {
            ChunkState::free(self.space_index)
        };
        // Do nothing if the chunk is already in the expected state.
        if self.get_any(chunk) == state {
            return;
        }
        #[cfg(debug_assertions)]
        {
            let old_state = self.get_any(chunk);
            // If a chunk is free, any space may use it. If a chunk is not free, only the current space may update its state.
            assert!(
                old_state.is_free() || old_state.get_space_index() == state.get_space_index(),
                "Chunk {:?}: old state {:?}, new state {:?}. Cannot set to new state.",
                chunk,
                old_state,
                state
            );
        }
        // Update alloc byte
        unsafe { Self::ALLOC_TABLE.store::<u8>(chunk.start(), state.0) };
        // If this is a newly allcoated chunk, then expand the chunk range.
        if state.is_allocated() {
            debug_assert!(!chunk.start().is_zero());
            let mut range = self.chunk_range.lock();
            if range.start == Chunk::ZERO {
                // FIXME: what if we actually use the first chunk?
                range.start = chunk;
                range.end = chunk.next();
            } else if chunk < range.start {
                range.start = chunk;
            } else if range.end <= chunk {
                range.end = chunk.next();
            }
        }
    }

    /// Get chunk state. Return None if the chunk does not belong to the space.
    pub fn get(&self, chunk: Chunk) -> Option<ChunkState> {
        let state = self.get_any(chunk);
        (state.get_space_index() == self.space_index).then_some(state)
    }

    /// Get chunk state, regardless of the space. This should always be private.
    fn get_any(&self, chunk: Chunk) -> ChunkState {
        let byte = unsafe { Self::ALLOC_TABLE.load::<u8>(chunk.start()) };
        ChunkState(byte)
    }

    /// A range of all chunks in the heap.
    pub fn all_chunks(&self) -> impl Iterator<Item = Chunk> + use<'_> {
        let chunk_range = self.chunk_range.lock();
        RegionIterator::<Chunk>::new(chunk_range.start, chunk_range.end)
            .filter(|c| self.get(*c).is_some())
    }

    /// A range of all chunks in the heap.
    pub fn all_allocated_chunks(&self) -> impl Iterator<Item = Chunk> + use<'_> {
        let chunk_range = self.chunk_range.lock();
        RegionIterator::<Chunk>::new(chunk_range.start, chunk_range.end)
            .filter(|c| self.get(*c).is_some_and(|state| state.is_allocated()))
    }

    /// Helper function to create per-chunk processing work packets for each allocated chunks.
    pub fn generate_tasks<VM: VMBinding>(
        &self,
        func: impl Fn(Chunk) -> Box<dyn GCWork<VM>>,
    ) -> Vec<Box<dyn GCWork<VM>>> {
        let mut work_packets: Vec<Box<dyn GCWork<VM>>> = vec![];
        for chunk in self.all_allocated_chunks() {
            work_packets.push(func(chunk));
        }
        work_packets
    }
}
