use std::{iter::Step, ops::Range, sync::atomic::AtomicU8};
use std::sync::atomic::{Ordering, AtomicUsize};
use crate::{MMTK, scheduler::*, util::{Address, ObjectReference, heap::layout::vm_layout_constants::{LOG_BYTES_IN_CHUNK, LOG_SPACE_EXTENT}}, vm::*};
use crate::util::side_metadata::{self, *};

use super::immixspace::ImmixSpace;
use super::block::{Block, BlockState};



#[repr(C)]
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, Eq)]
pub struct Chunk(Address);

impl Chunk {
    pub const LOG_BYTES: usize = LOG_BYTES_IN_CHUNK;
    pub const BYTES: usize = 1 << Self::LOG_BYTES;
    pub const LOG_BLOCKS: usize = Self::LOG_BYTES - Block::LOG_BYTES;
    pub const BLOCKS: usize = 1 << Self::LOG_BLOCKS;


    pub const fn align(address: Address) -> Address {
        address.align_down(Self::BYTES)
    }

    pub const fn is_aligned(address: Address) -> bool {
        Self::align(address).as_usize() == address.as_usize()
    }

    pub const fn from(address: Address) -> Self {
        debug_assert!(address.is_aligned_to(Self::BYTES));
        Self(address)
    }

    #[inline(always)]
    pub fn containing<VM: VMBinding>(object: ObjectReference) -> Self {
        Self(VM::VMObjectModel::ref_to_address(object).align_down(Self::BYTES))
    }

    pub const fn start(&self) -> Address {
        self.0
    }

    pub const fn end(&self) -> Address {
        unsafe { Address::from_usize(self.0.as_usize() + Self::BYTES) }
    }

    pub const fn blocks(&self) -> Range<Block> {
        let start = Block::from(Block::align(self.0));
        let end = Block::from(start.start() + (Self::BLOCKS << Block::LOG_BYTES));
        start..end
    }

    pub fn sweep<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        if super::BLOCK_ONLY {
            let mut allocated_blocks = 0;
            for block in self.blocks() {
                match block.get_state() {
                    BlockState::Unallocated => {},
                    BlockState::Unmarked => {
                        space.release_block(block);
                    },
                    BlockState::Marked => {
                        allocated_blocks += 1;
                    }
                    _ => unreachable!()
                }
                if block.get_state() == BlockState::Unmarked {
                    space.release_block(block);
                }
            }
            // Remove this chunk if there are no live blocks
            if allocated_blocks == 0 {
                space.chunk_map.set(*self, ChunkState::Free)
            }
        } else {
            for block in self.blocks().filter(|block| block.get_state() != BlockState::Unallocated) {
                let mut marked_lines = 0;
                for line in block.lines() {
                    if line.is_marked(space.line_mark_state.load(Ordering::SeqCst)) {
                        marked_lines += 1;
                    }
                }
                if marked_lines == 0 {
                    space.release_block(block);
                } else if marked_lines != Block::LINES {
                    space.reusable_blocks.push(block)
                }
            }
        }
    }
}

unsafe impl Step for Chunk {
    #[inline(always)]
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        if start < end { return None }
        Some((end.start() - start.start()) >> Self::LOG_BYTES)
    }
    #[inline(always)]
    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Self::from(start.start() + (count << Self::LOG_BYTES)))
    }
    #[inline(always)]
    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        Some(Self::from(start.start() - (count << Self::LOG_BYTES)))
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq)]
pub enum ChunkState {
    Free = 0,
    Allocated = 1,
}

pub struct ChunkMap {
    table: Vec<AtomicU8>,
    start: Address,
}

impl ChunkMap {
    pub const MAX_CHUNKS: usize = 1 << (LOG_SPACE_EXTENT - Chunk::LOG_BYTES);

    pub fn new(start: Address) -> Self {
        Self {
            table: (0..Self::MAX_CHUNKS).map(|_| Default::default()).collect(),
            start,
        }
    }

    const fn get_index(&self, chunk: Chunk) -> usize {
        // let space_start = chunk.start().as_usize() & ((1 << LOG_SPACE_EXTENT) - 1);
        (chunk.start().as_usize() - self.start.as_usize()) >> Chunk::LOG_BYTES
    }

    pub fn set(&self, chunk: Chunk, state: ChunkState) {
        let index = self.get_index(chunk);
        self.table[index].store(state as _, Ordering::SeqCst);
    }

    pub fn get(&self, chunk: Chunk) -> ChunkState {
        let index = self.get_index(chunk);
        let byte = self.table[index].load(Ordering::SeqCst);
        unsafe { std::mem::transmute(byte) }
    }

    pub fn all_chunks(&self) -> Range<Chunk> {
        let start = Chunk::from(self.start);
        let end = Chunk::forward(start, Self::MAX_CHUNKS);
        start..end
    }

    pub fn allocated_chunks<'a>(&'a self) -> impl Iterator<Item=Chunk> + 'a {
        AllocatedChunksIter {
            table: &self.table,
            start: self.start,
            cursor: 0,
        }
    }

    pub fn generate_sweep_tasks<VM: VMBinding>(&self, space: &'static ImmixSpace<VM>, mmtk: &'static MMTK<VM>) -> Vec<Box<dyn Work<MMTK<VM>>>> {
        let Range { start: start_chunk, end: end_chunk } = self.all_chunks();
        let workers = mmtk.scheduler.worker_group().worker_count() * 2;
        let chunks_per_packet = (ChunkMap::MAX_CHUNKS + (workers - 1)) / workers;
        let mut work_packets: Vec<Box<dyn Work<MMTK<VM>>>> = vec![];
        for start in (start_chunk..end_chunk).step_by(chunks_per_packet) {
            let mut end = Chunk::forward(start, chunks_per_packet);
            if end > end_chunk {
                end = end_chunk;
            }
            work_packets.push(box SweepChunks(space, start..end));
        }
        work_packets
    }
}

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
            let state = self.table[self.cursor].load(Ordering::SeqCst);
            let cursor = self.cursor;
            self.cursor += 1;
            if state == 1 {
                return Some(Chunk::from(self.start + (cursor << Chunk::LOG_BYTES)));
            }
        }
        None
    }
}

pub struct SweepChunks<VM: VMBinding>(pub &'static ImmixSpace<VM>, pub Range<Chunk>);

impl<VM: VMBinding> GCWork<VM> for SweepChunks<VM> {
    #[inline]
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        for chunk in self.1.start..self.1.end {
            if self.0.chunk_map.get(chunk) == ChunkState::Allocated {
                chunk.sweep(self.0);
            }
        }
    }
}
