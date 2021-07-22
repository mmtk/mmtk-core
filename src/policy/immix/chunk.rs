use super::block::{Block, BlockState};
use super::defrag::MarkHistogram;
use super::immixspace::ImmixSpace;
use crate::util::metadata::side_metadata::{self, SideMetadataSpec};
use crate::{
    scheduler::*,
    util::{heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK, Address},
    vm::*,
    MMTK,
};
use spin::Mutex;
use std::{iter::Step, ops::Range, sync::atomic::Ordering};

/// Data structure to reference a MMTk 4 MB chunk.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Chunk(Address);

impl Chunk {
    /// Chunk constant with zero address
    const ZERO: Self = Self(Address::ZERO);
    /// Log bytes in chunk
    pub const LOG_BYTES: usize = LOG_BYTES_IN_CHUNK;
    /// Bytes in chunk
    pub const BYTES: usize = 1 << Self::LOG_BYTES;
    /// Log blocks in chunk
    pub const LOG_BLOCKS: usize = Self::LOG_BYTES - Block::LOG_BYTES;
    /// Blocks in chunk
    pub const BLOCKS: usize = 1 << Self::LOG_BLOCKS;

    /// Align the give address to the chunk boundary.
    pub const fn align(address: Address) -> Address {
        address.align_down(Self::BYTES)
    }

    /// Get the chunk from a given address.
    /// The address must be chunk-aligned.
    #[inline(always)]
    pub fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    /// Get chunk start address
    pub const fn start(&self) -> Address {
        self.0
    }

    /// Get a range of blocks within this chunk.
    #[inline(always)]
    pub fn blocks(&self) -> Range<Block> {
        let start = Block::from(Block::align(self.0));
        let end = Block::from(start.start() + (Self::BLOCKS << Block::LOG_BYTES));
        start..end
    }

    /// Sweep this chunk.
    pub fn sweep<VM: VMBinding>(&self, space: &ImmixSpace<VM>, mark_histogram: &mut MarkHistogram) {
        let mut allocated_blocks = 0; // number of allocated blocks.
        if super::BLOCK_ONLY {
            // Iterate over all blocks in this chunk.
            for block in self.blocks() {
                match block.get_state() {
                    BlockState::Unallocated => {}
                    BlockState::Unmarked => {
                        // Release the block if it is allocated but not marked by the current GC.
                        space.release_block(block);
                    }
                    BlockState::Marked => {
                        // The block is live. Update counter.
                        allocated_blocks += 1;
                    }
                    _ => unreachable!(),
                }
            }
        } else {
            let line_mark_state = space.line_mark_state.load(Ordering::Acquire);
            // Iterate over all allocated blocks in this chunk.
            for block in self
                .blocks()
                .filter(|block| block.get_state() != BlockState::Unallocated)
            {
                // Calculate number of marked lines and holes.
                let mut marked_lines = 0;
                let mut holes = 0;
                let mut prev_line_is_marked = true;

                for line in block.lines() {
                    if line.is_marked(line_mark_state) {
                        marked_lines += 1;
                        prev_line_is_marked = true;
                    } else {
                        if prev_line_is_marked {
                            holes += 1;
                        }
                        prev_line_is_marked = false;
                    }
                }

                if marked_lines == 0 {
                    // Release the block if non of its lines are marked.
                    space.release_block(block);
                } else {
                    // There are some marked lines. Keep the block live and update counter.
                    allocated_blocks += 1;
                    if marked_lines != Block::LINES {
                        // There are holes. Mark the block as reusable.
                        block.set_state(BlockState::Reusable {
                            unavailable_lines: marked_lines as _,
                        });
                        space.reusable_blocks.push(block)
                    } else {
                        // Clear mark state.
                        block.set_state(BlockState::Unmarked);
                    }
                    // Update mark_histogram
                    mark_histogram[holes] += marked_lines;
                    // Record number of holes in block side metadata.
                    block.set_holes(holes);
                }
            }
        }
        // Set this chunk as free if there is not live blocks.
        if allocated_blocks == 0 {
            space.chunk_map.set(*self, ChunkState::Free)
        }
    }
}

unsafe impl Step for Chunk {
    /// Get the number of chunks between the given two chunks.
    #[inline(always)]
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        if start > end {
            return None;
        }
        Some((end.start() - start.start()) >> Self::LOG_BYTES)
    }
    /// result = chunk_address + count * chunk_size
    #[inline(always)]
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Self::from(start.start() + (count << Self::LOG_BYTES)))
    }
    /// result = chunk_address - count * chunk_size
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Self::from(start.start() - (count << Self::LOG_BYTES)))
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
    pub const ALLOC_TABLE: SideMetadataSpec = SideMetadataSpec {
        is_global: false,
        offset: Block::MARK_TABLE.accumulated_size(),
        log_num_of_bits: 3,
        log_min_obj_size: Chunk::LOG_BYTES,
    };

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
        unsafe { side_metadata::store(&Self::ALLOC_TABLE, chunk.start(), state as u8 as _) };
        // If this is a newly allcoated chunk, then expand the chunk range.
        if state == ChunkState::Allocated {
            debug_assert!(!chunk.start().is_zero());
            let mut range = self.chunk_range.lock();
            if range.start == Chunk::ZERO {
                range.start = chunk;
                range.end = Chunk::forward(chunk, 1);
            } else if chunk < range.start {
                range.start = chunk;
            } else if range.end <= chunk {
                range.end = Chunk::forward(chunk, 1);
            }
        }
    }

    /// Get chunk state
    pub fn get(&self, chunk: Chunk) -> ChunkState {
        let byte = unsafe { side_metadata::load(&Self::ALLOC_TABLE, chunk.start()) as u8 };
        match byte {
            0 => ChunkState::Free,
            1 => ChunkState::Allocated,
            _ => unreachable!(),
        }
    }

    /// A range of all chunks in the heap.
    pub fn all_chunks(&self) -> Range<Chunk> {
        self.chunk_range.lock().clone()
    }

    /// Helper function to create per-chunk processing work packets.
    pub fn generate_tasks<VM: VMBinding>(
        &self,
        func: impl Fn(Chunk) -> Box<dyn Work<MMTK<VM>>>,
    ) -> Vec<Box<dyn Work<MMTK<VM>>>> {
        let mut work_packets: Vec<Box<dyn Work<MMTK<VM>>>> = vec![];
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
    ) -> Vec<Box<dyn Work<MMTK<VM>>>> {
        space.defrag.mark_histograms.lock().clear();
        self.generate_tasks(|chunk| box SweepChunk { space, chunk })
    }
}

/// Chunk sweeping work packet.
struct SweepChunk<VM: VMBinding> {
    space: &'static ImmixSpace<VM>,
    chunk: Chunk,
}

impl<VM: VMBinding> GCWork<VM> for SweepChunk<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        let mut histogram = self.space.defrag.new_mark_histogram();
        if self.space.chunk_map.get(self.chunk) == ChunkState::Allocated {
            self.chunk.sweep(self.space, &mut histogram);
        }
        self.space.defrag.add_completed_mark_histogram(histogram);
    }
}
