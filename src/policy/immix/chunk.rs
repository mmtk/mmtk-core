use super::block::{Block, BlockState};
use super::defrag::Histogram;
use super::immixspace::ImmixSpace;
use crate::util::linear_scan::{Region, RegionIterator};
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::{
    scheduler::*,
    util::{heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK, Address},
    vm::*,
    MMTK,
};
use spin::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::{ops::Range, sync::atomic::Ordering};

/// Data structure to reference a MMTk 4 MB chunk.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Chunk(Address);

impl Region for Chunk {
    const LOG_BYTES: usize = LOG_BYTES_IN_CHUNK;

    #[inline(always)]
    fn from_aligned_address(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    #[inline(always)]
    fn start(&self) -> Address {
        self.0
    }
}

impl Chunk {
    /// Chunk constant with zero address
    const ZERO: Self = Self(Address::ZERO);
    /// Log blocks in chunk
    pub const LOG_BLOCKS: usize = Self::LOG_BYTES - Block::LOG_BYTES;
    /// Blocks in chunk
    pub const BLOCKS: usize = 1 << Self::LOG_BLOCKS;

    /// Get a range of blocks within this chunk.
    #[inline(always)]
    pub fn blocks(&self) -> RegionIterator<Block> {
        let start = Block::from_unaligned_address(self.0);
        let end = Block::from_aligned_address(start.start() + (Self::BLOCKS << Block::LOG_BYTES));
        RegionIterator::<Block>::new(start, end)
    }

    /// Sweep this chunk.
    pub fn sweep<VM: VMBinding>(&self, space: &ImmixSpace<VM>, mark_histogram: &mut Histogram) {
        let line_mark_state = if super::BLOCK_ONLY {
            None
        } else {
            Some(space.line_mark_state.load(Ordering::Acquire))
        };
        // number of allocated blocks.
        let mut allocated_blocks = 0;
        // Iterate over all allocated blocks in this chunk.
        for block in self
            .blocks()
            .filter(|block| block.get_state() != BlockState::Unallocated)
        {
            if !block.sweep(space, mark_histogram, line_mark_state) {
                // Block is live. Increment the allocated block count.
                allocated_blocks += 1;
            }
        }
        // Set this chunk as free if there is not live blocks.
        if allocated_blocks == 0 {
            space.chunk_map.set(*self, ChunkState::Free)
        }
    }
}

/// Chunk allocation state
#[repr(u8)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ChunkState {
    /// The chunk is not allocated.
    Free = 0,
    /// The chunk is allocated.
    Allocated = 1,
}

/// A byte-map to record all the allocated chunks
pub struct ChunkMap {
    chunk_range: Mutex<Range<Chunk>>,
}

impl ChunkMap {
    /// Chunk alloc table
    pub const ALLOC_TABLE: SideMetadataSpec =
        crate::util::metadata::side_metadata::spec_defs::IX_CHUNK_MARK;

    pub fn new() -> Self {
        Self {
            chunk_range: Mutex::new(Chunk::ZERO..Chunk::ZERO),
        }
    }

    /// Set chunk state
    pub fn set(&self, chunk: Chunk, state: ChunkState) {
        // Do nothing if the chunk is already in the expected state.
        if self.get(chunk) == state {
            return;
        }
        // Update alloc byte
        unsafe { Self::ALLOC_TABLE.store::<u8>(chunk.start(), state as u8) };
        // If this is a newly allcoated chunk, then expand the chunk range.
        if state == ChunkState::Allocated {
            debug_assert!(!chunk.start().is_zero());
            let mut range = self.chunk_range.lock();
            if range.start == Chunk::ZERO {
                range.start = chunk;
                range.end = chunk.next();
            } else if chunk < range.start {
                range.start = chunk;
            } else if range.end <= chunk {
                range.end = chunk.next();
            }
        }
    }

    /// Get chunk state
    pub fn get(&self, chunk: Chunk) -> ChunkState {
        let byte = unsafe { Self::ALLOC_TABLE.load::<u8>(chunk.start()) };
        match byte {
            0 => ChunkState::Free,
            1 => ChunkState::Allocated,
            _ => unreachable!(),
        }
    }

    /// A range of all chunks in the heap.
    pub fn all_chunks(&self) -> RegionIterator<Chunk> {
        let chunk_range = self.chunk_range.lock();
        RegionIterator::<Chunk>::new(chunk_range.start, chunk_range.end)
    }

    /// Helper function to create per-chunk processing work packets.
    pub fn generate_tasks<VM: VMBinding>(
        &self,
        func: impl Fn(Chunk) -> Box<dyn GCWork<VM>>,
    ) -> Vec<Box<dyn GCWork<VM>>> {
        let mut work_packets: Vec<Box<dyn GCWork<VM>>> = vec![];
        for chunk in self
            .all_chunks()
            .filter(|c| self.get(*c) == ChunkState::Allocated)
        {
            work_packets.push(func(chunk));
        }
        work_packets
    }

    /// Generate chunk sweep work packets.
    pub fn generate_sweep_tasks<VM: VMBinding>(
        &self,
        space: &'static ImmixSpace<VM>,
    ) -> Vec<Box<dyn GCWork<VM>>> {
        space.defrag.mark_histograms.lock().clear();
        let epilogue = Arc::new(FlushPageResource {
            space,
            counter: AtomicUsize::new(0),
        });
        let tasks = self.generate_tasks(|chunk| {
            Box::new(SweepChunk {
                space,
                chunk,
                epilogue: epilogue.clone(),
            })
        });
        epilogue.counter.store(tasks.len(), Ordering::SeqCst);
        tasks
    }
}

impl Default for ChunkMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Chunk sweeping work packet.
struct SweepChunk<VM: VMBinding> {
    space: &'static ImmixSpace<VM>,
    chunk: Chunk,
    /// A destructor invoked when all `SweepChunk` packets are finished.
    epilogue: Arc<FlushPageResource<VM>>,
}

impl<VM: VMBinding> GCWork<VM> for SweepChunk<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let mut histogram = self.space.defrag.new_histogram();
        if self.space.chunk_map.get(self.chunk) == ChunkState::Allocated {
            self.chunk.sweep(self.space, &mut histogram);
        }
        self.space.defrag.add_completed_mark_histogram(histogram);
        self.epilogue.finish_one_work_packet();
    }
}

/// Count number of remaining work pacets, and flush page resource if all packets are finished.
struct FlushPageResource<VM: VMBinding> {
    space: &'static ImmixSpace<VM>,
    counter: AtomicUsize,
}

impl<VM: VMBinding> FlushPageResource<VM> {
    /// Called after a related work packet is finished.
    fn finish_one_work_packet(&self) {
        if 1 == self.counter.fetch_sub(1, Ordering::SeqCst) {
            // We've finished releasing all the dead blocks to the BlockPageResource's thread-local queues.
            // Now flush the BlockPageResource.
            self.space.flush_page_resource()
        }
    }
}
