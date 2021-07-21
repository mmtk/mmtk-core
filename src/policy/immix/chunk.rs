use super::block::{Block, BlockState};
use super::immixspace::ImmixSpace;
use crate::{
    scheduler::*,
    util::{
        heap::layout::vm_layout_constants::{LOG_BYTES_IN_CHUNK, MAX_CHUNKS},
        Address, ObjectReference,
    },
    vm::*,
    MMTK,
};
use std::{
    iter::Step,
    ops::Range,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
};

/// Data structure to reference a MMTk 4 MB chunk.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Chunk(Address);

impl Chunk {
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

    /// Test if the given address is chunk-aligned
    pub const fn is_aligned(address: Address) -> bool {
        Self::align(address).as_usize() == address.as_usize()
    }

    /// Get the chunk from a given address.
    /// The address must be chunk-aligned.
    #[inline(always)]
    pub fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    /// Get the chunk containing the given address.
    /// The input address does not need to be aligned.
    #[inline(always)]
    pub fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self(VM::VMObjectModel::ref_to_address(object).align_down(Self::BYTES))
    }

    /// Get chunk start address
    pub const fn start(&self) -> Address {
        self.0
    }

    /// Get chunk end address
    pub const fn end(&self) -> Address {
        self.0.add( Self::BYTES)
    }

    /// Get a range of blocks within this chunk.
    #[inline(always)]
    pub fn blocks(&self) -> Range<Block> {
        let start = Block::from(Block::align(self.0));
        let end = Block::from(start.start() + (Self::BLOCKS << Block::LOG_BYTES));
        start..end
    }

    /// Sweep this chunk.
    pub fn sweep<VM: VMBinding>(&self, space: &ImmixSpace<VM>, mark_histogram: &[AtomicUsize]) {
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
                    let old_value = mark_histogram[holes].load(Ordering::Relaxed);
                    mark_histogram[holes].store(old_value + marked_lines, Ordering::Relaxed);
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
#[derive(Debug, PartialEq)]
pub enum ChunkState {
    /// The chunk is not allocated.
    Free = 0,
    /// The chunk is allocated.
    Allocated = 1,
}

/// A byte-map to record all the allocated chunks
pub struct ChunkMap {
    table: Vec<AtomicU8>,
    start: Address,
    limit: AtomicUsize,
}

impl ChunkMap {
    pub fn new(start: Address) -> Self {
        Self {
            table: (0..MAX_CHUNKS).map(|_| Default::default()).collect(),
            start,
            limit: AtomicUsize::new(0),
        }
    }

    /// Get the index of the chunk.
    const fn get_index(&self, chunk: Chunk) -> usize {
        chunk.start().get_extent(self.start) >> Chunk::LOG_BYTES
    }

    /// Set chunk state
    pub fn set(&self, chunk: Chunk, state: ChunkState) {
        let index = self.get_index(chunk);
        if state == ChunkState::Allocated {
            let _ = self
                .limit
                .fetch_update(Ordering::Release, Ordering::Relaxed, |old| {
                    if index + 1 > old {
                        Some(index + 1)
                    } else {
                        None
                    }
                });
        }
        self.table[index].store(state as _, Ordering::Release);
    }

    /// Get chunk state
    pub fn get(&self, chunk: Chunk) -> ChunkState {
        let index = self.get_index(chunk);
        let byte = self.table[index].load(Ordering::Acquire);
        unsafe { std::mem::transmute(byte) }
    }

    /// A range of all chunks in the heap.
    pub fn all_chunks(&self) -> Range<Chunk> {
        let start = Chunk::from(self.start);
        let end = Chunk::forward(start, self.limit.load(Ordering::Acquire));
        start..end
    }

    /// A iterator of all the *allocated* chunks.
    pub fn allocated_chunks(&'_ self) -> impl Iterator<Item = Chunk> + '_ {
        AllocatedChunksIter {
            table: &self.table,
            start: self.start,
            cursor: 0,
        }
    }

    /// Helper function to create per-chunk processing work packets.
    pub fn generate_tasks<VM: VMBinding>(
        &self,
        workers: usize,
        func: impl Fn(Range<Chunk>) -> Box<dyn Work<MMTK<VM>>>,
    ) -> Vec<Box<dyn Work<MMTK<VM>>>> {
        let Range {
            start: start_chunk,
            end: end_chunk,
        } = self.all_chunks();
        let chunks = Chunk::steps_between(&start_chunk, &end_chunk).unwrap();
        let chunks_per_packet = (chunks + (workers * 2 - 1)) / workers;
        let mut work_packets: Vec<Box<dyn Work<MMTK<VM>>>> = vec![];
        for start in (start_chunk..end_chunk).step_by(chunks_per_packet) {
            let mut end = Chunk::forward(start, chunks_per_packet);
            if end > end_chunk {
                end = end_chunk;
            }
            work_packets.push(func(start..end));
        }
        work_packets
    }

    /// Generate chunk sweep work packets.
    pub fn generate_sweep_tasks<VM: VMBinding>(
        &self,
        space: &'static ImmixSpace<VM>,
        scheduler: &MMTkScheduler<VM>,
    ) -> Vec<Box<dyn Work<MMTK<VM>>>> {
        for table in space.defrag.spill_mark_histograms() {
            for entry in table {
                entry.store(0, Ordering::Release);
            }
        }
        self.generate_tasks(scheduler.num_workers(), |chunks| {
            box SweepChunks(space, chunks)
        })
    }
}

/// Iterator to iterate over all allocated chunks.
struct AllocatedChunksIter<'a> {
    table: &'a [AtomicU8],
    start: Address,
    cursor: usize,
}

impl<'a> Iterator for AllocatedChunksIter<'a> {
    type Item = Chunk;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        while self.cursor < self.table.len() {
            let state = self.table[self.cursor].load(Ordering::Acquire);
            let cursor = self.cursor;
            self.cursor += 1;
            if state == 1 {
                return Some(Chunk::from(self.start + (cursor << Chunk::LOG_BYTES)));
            }
        }
        None
    }
}

/// Chunk sweeping work packet.
pub struct SweepChunks<VM: VMBinding>(pub &'static ImmixSpace<VM>, pub Range<Chunk>);

impl<VM: VMBinding> GCWork<VM> for SweepChunks<VM> {
    #[inline]
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        for chunk in self.1.start..self.1.end {
            if self.0.chunk_map.get(chunk) == ChunkState::Allocated {
                chunk.sweep(
                    self.0,
                    &self.0.defrag.spill_mark_histograms()[worker.ordinal],
                );
            }
        }
    }
}
