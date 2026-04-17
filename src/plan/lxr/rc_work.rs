use std::{
    ops::Range,
    sync::{atomic::AtomicUsize, Mutex},
};

use atomic::Ordering;
use crossbeam::queue::SegQueue;

use crate::{
    plan::lxr::LazySweepingJobsCounter,
    plan::{immix::Pause, lxr::LXR},
    scheduler::{GCWork, GCWorker, WorkBucketStage},
    util::{
        constants::LOG_BYTES_IN_PAGE,
        heap::chunk_map::Chunk,
        linear_scan::Region,
        rc::{self, RefCountHelper},
        ObjectReference,
    },
    vm::{ObjectModel, VMBinding},
    Plan, MMTK,
};

use crate::policy::immix::{
    block::{Block, BlockState},
    line::Line,
    ImmixSpace,
};

static SELECT_DEFRAG_BLOCK_JOB_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct SelectDefragBlocks {
    pub chunks: Range<Chunk>,
    #[allow(unused)]
    pub defrag_threshold: usize,
}

impl<VM: VMBinding> GCWork<VM> for SelectDefragBlocks {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let mut fragmented_blocks = vec![];
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();

        // Iterate over all blocks in this chunk
        let num_chunks = (self.chunks.end.start() - self.chunks.start.start()) >> Chunk::LOG_BYTES;
        let ix_space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        for i in 0..num_chunks {
            let chunk = self.chunks.start.next_nth(i);
            if !ix_space.chunk_map.is_allocated(chunk) {
                continue;
            }
            for block in chunk.iter_region::<Block>() {
                // Skip unallocated blocks.
                if MatureEvacuationSet::skip_block(block) {
                    continue;
                }
                // This is a fragmented block?
                let score = block.calc_dead_lines() << Line::LOG_BYTES;
                if lxr.current_pause().unwrap() == Pause::Full || score >= (Block::BYTES >> 1) {
                    fragmented_blocks.push((block, score));
                }
            }
        }
        // Flush to global fragmented_blocks
        if !fragmented_blocks.is_empty() {
            lxr.immix_space
                .evac_set
                .fragmented_blocks_size
                .fetch_add(fragmented_blocks.len(), Ordering::SeqCst);
            lxr.immix_space
                .evac_set
                .fragmented_blocks
                .push(fragmented_blocks);
        }

        if SELECT_DEFRAG_BLOCK_JOB_COUNTER.fetch_sub(1, Ordering::SeqCst) == 1 {
            lxr.immix_space
                .evac_set
                .select_mature_evacuation_candidates(
                    lxr,
                    lxr.current_pause().unwrap(),
                    mmtk.get_plan().get_total_pages(),
                )
        }
    }
}

pub(crate) struct SweepBlocksAfterDecs {
    blocks: Vec<(Block, bool)>,
    _counter: LazySweepingJobsCounter,
}

impl SweepBlocksAfterDecs {
    pub(crate) fn new(blocks: Vec<(Block, bool)>, counter: LazySweepingJobsCounter) -> Self {
        Self {
            blocks,
            _counter: counter,
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for SweepBlocksAfterDecs {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        if self.blocks.is_empty() {
            return;
        }
        let mut count = 0;
        for (block, defrag) in &self.blocks {
            block.unlog();
            if block.rc_sweep_mature::<VM>(&lxr.immix_space, *defrag) {
                count += 1;
            } else {
                assert!(
                    !*defrag,
                    "defrag block is freed? {:?} {:?} {}",
                    block,
                    block.get_state(),
                    block.is_defrag_source()
                );
            }
        }
        if count != 0
            && (lxr.current_pause().is_none()
                || mmtk.scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].is_open())
        {
            lxr.immix_space
                .num_clean_blocks_released_lazy
                .fetch_add(count, Ordering::Relaxed);
        }
    }
}

/// Chunk sweeping work packet.
pub(crate) struct SweepDeadCycles<VM: VMBinding> {
    chunks: Range<Chunk>,
    _counter: LazySweepingJobsCounter,
    rc: RefCountHelper<VM>,
}

#[allow(unused)]
impl<VM: VMBinding> SweepDeadCycles<VM> {
    const CAPACITY: usize = 1024;

    pub(crate) fn new(chunks: Range<Chunk>, counter: LazySweepingJobsCounter) -> Self {
        Self {
            chunks,
            _counter: counter,
            rc: RefCountHelper::NEW,
        }
    }

    fn process_dead_object(&mut self, o: ObjectReference) {
        if RefCountHelper::<VM>::SANITY {
            unsafe {
                o.to_raw_address().store(0xdeadusize);
            }
        }
        self.rc.unmark_straddle_object(o);
        self.rc.set(o, 0);
    }

    fn process_block(&mut self, block: Block, immix_space: &ImmixSpace<VM>) {
        let mut has_dead_object = false;
        let mut has_live = false;
        let mut cursor = block.start();
        let limit = block.end();
        while cursor < limit {
            let o = unsafe { cursor.to_object_reference::<VM>() };
            cursor = cursor + rc::MIN_OBJECT_SIZE;
            let c = self.rc.count(o);
            if c != 0 && !immix_space.is_marked(o) {
                if Line::is_aligned(o.to_raw_address()) {
                    if c == 1 && self.rc.is_straddle_line(Line::from(o.to_raw_address())) {
                        continue;
                    } else {
                        std::sync::atomic::fence(Ordering::SeqCst);
                        if self.rc.count(o) == 0 {
                            continue;
                        }
                    }
                }
                self.process_dead_object(o);
                has_dead_object = true;
            } else {
                if c != 0 {
                    has_live = true;
                }
            }
        }
        if has_dead_object || !has_live {
            immix_space.add_to_possibly_dead_mature_blocks(block, false);
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for SweepDeadCycles<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        let immix_space = &lxr.immix_space;
        let num_chunks = (self.chunks.end.start() - self.chunks.start.start()) >> Chunk::LOG_BYTES;
        let ix_space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        for i in 0..num_chunks {
            let chunk = self.chunks.start.next_nth(i);
            if !ix_space.chunk_map.is_allocated(chunk) {
                continue;
            }

            for block in chunk
                .iter_region::<Block>()
                .filter(|block| block.get_state() != BlockState::Unallocated)
            {
                if block.is_defrag_source() || block.get_state() == BlockState::Nursery {
                    continue;
                } else {
                    self.process_block(block, immix_space)
                }
            }
        }
    }
}

pub(crate) struct ConcurrentChunkMetadataZeroing {
    pub chunks: Range<Chunk>,
}

impl ConcurrentChunkMetadataZeroing {
    /// Clear object mark table
    #[allow(unused)]
    fn reset_object_mark<VM: VMBinding>(chunk: Chunk) {
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
            .extract_side_spec()
            .bzero_metadata(chunk.start(), Chunk::BYTES);
    }
}

impl<VM: VMBinding> GCWork<VM> for ConcurrentChunkMetadataZeroing {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let num_chunks = (self.chunks.end.start() - self.chunks.start.start()) >> Chunk::LOG_BYTES;
        let ix_space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        for i in 0..num_chunks {
            let chunk = self.chunks.start.next_nth(i);
            if !ix_space.chunk_map.is_allocated(chunk) {
                continue;
            }
            Self::reset_object_mark::<VM>(chunk);
        }
    }
}

/// A work packet to prepare each block for GC.
/// Performs the action on a range of chunks.
pub(crate) struct PrepareChunksForFullGC {
    pub chunks: Range<Chunk>,
}

impl PrepareChunksForFullGC {
    /// Clear object mark table
    #[allow(unused)]
    fn reset_object_mark<VM: VMBinding>(chunk: Chunk) {
        VM::VMObjectModel::LOCAL_MARK_BIT_SPEC
            .extract_side_spec()
            .bzero_metadata(chunk.start(), Chunk::BYTES);
    }
}

impl<VM: VMBinding> GCWork<VM> for PrepareChunksForFullGC {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let num_chunks = (self.chunks.end.start() - self.chunks.start.start()) >> Chunk::LOG_BYTES;
        let ix_space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        for i in 0..num_chunks {
            let chunk = self.chunks.start.next_nth(i);
            if !ix_space.chunk_map.is_allocated(chunk) {
                continue;
            }
            // Iterate over all blocks in this chunk
            for block in chunk.iter_region::<Block>() {
                let state = block.get_state();
                // Skip unallocated blocks.
                if state == BlockState::Unallocated {
                    continue;
                }
                // Clear defrag state
                assert!(!block.is_defrag_source());
                // Clear block mark data.
                if block.get_state() != BlockState::Nursery {
                    block.set_state(BlockState::Unmarked);
                }
                debug_assert!(!block.get_state().is_reusable());
                // debug_assert_ne!(block.get_state(), BlockState::Marked);
                // debug_assert_ne!(block.get_state(), BlockState::Nursery);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct MatureEvacuationSet {
    fragmented_blocks: SegQueue<Vec<(Block, usize)>>,
    fragmented_blocks_size: AtomicUsize,
    blocks_in_fragmented_chunks: SegQueue<Vec<(Block, usize)>>,
    blocks_in_fragmented_chunks_size: AtomicUsize,
    defrag_blocks: Mutex<Vec<Block>>,
    num_defrag_blocks: AtomicUsize,
}

impl MatureEvacuationSet {
    /// Release all the mature defrag source blocks
    pub(crate) fn sweep_mature_evac_candidates<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        let mut defrag_blocks: Vec<Block> =
            std::mem::take(&mut *self.defrag_blocks.lock().unwrap());
        if defrag_blocks.is_empty() {
            return;
        }
        while let Some(block) = defrag_blocks.pop() {
            if !block.is_defrag_source() || block.get_state() == BlockState::Unallocated {
                // This block has been eagerly released (probably be reused again). Skip it.
                continue;
            }
            block.clear_rc_table::<VM>();
            block.clear_striddle_table::<VM>();
            block.rc_sweep_mature::<VM>(space, true);
            assert!(!block.is_defrag_source());
        }
    }

    pub(crate) fn schedule_defrag_selection_packets<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        let tasks = space.chunk_map.generate_tasks_batched(|chunks| {
            Box::new(SelectDefragBlocks {
                chunks,
                defrag_threshold: 1,
            })
        });
        self.fragmented_blocks_size.store(0, Ordering::SeqCst);
        SELECT_DEFRAG_BLOCK_JOB_COUNTER.store(tasks.len(), Ordering::SeqCst);
        space.scheduler().work_buckets[WorkBucketStage::Unconstrained].bulk_add(tasks);
    }

    fn skip_block(b: Block) -> bool {
        let s = b.get_state();
        b.is_defrag_source() || s == BlockState::Unallocated || s == BlockState::Nursery
    }

    fn select_fragmented_blocks(
        &self,
        selected_blocks: &mut Vec<Block>,
        copy_bytes: &mut usize,
        max_copy_bytes: usize,
    ) {
        let mut blocks = Vec::with_capacity(self.fragmented_blocks_size.load(Ordering::SeqCst));
        while let Some(mut x) = self.fragmented_blocks.pop() {
            blocks.append(&mut x);
        }
        blocks.sort_by_key(|x| x.1);
        while let Some((block, _dead_bytes)) = blocks.pop() {
            if Self::skip_block(block) {
                continue;
            }
            block.set_as_defrag_source(true);
            selected_blocks.push(block);
            *copy_bytes += (Block::BYTES - (block.calc_dead_lines() << Line::LOG_BYTES)) >> 1;
            if *copy_bytes >= max_copy_bytes {
                break;
            }
        }
    }

    fn select_mature_evacuation_candidates<VM: VMBinding>(
        &self,
        lxr: &LXR<VM>,
        _pause: Pause,
        _total_pages: usize,
    ) {
        debug_assert!(crate::plan::lxr::MATURE_EVACUATION);
        if lxr.current_pause().unwrap() == Pause::Full {
            // Make sure LOS sweeping finishes before evac selection begin
            // FIXME: This can be done in parallel with SelectDefragBlocksInChunk packets
            let los = lxr.common().get_los();
            los.release_rc_nursery_objects();
        }
        // Select mature defrag blocks
        let available_clean_pages_for_defrag = if lxr.current_pause().unwrap() == Pause::Full {
            lxr.get_total_pages()
                .saturating_sub(lxr.get_used_pages())
                .max(lxr.immix_space.defrag_headroom_pages())
        } else {
            lxr.immix_space.defrag_headroom_pages()
        };
        let max_copy_bytes = available_clean_pages_for_defrag << LOG_BYTES_IN_PAGE;
        let mut copy_bytes = 0usize;
        let mut selected_blocks = vec![];
        self.select_fragmented_blocks(&mut selected_blocks, &mut copy_bytes, max_copy_bytes);
        self.num_defrag_blocks
            .store(selected_blocks.len(), Ordering::SeqCst);
        let mut defrag_blocks = self.defrag_blocks.lock().unwrap();
        *defrag_blocks = selected_blocks;
        // cleanup
        assert!(self.fragmented_blocks.is_empty());
        assert!(self.blocks_in_fragmented_chunks.is_empty());
        self.fragmented_blocks_size.store(0, Ordering::SeqCst);
        self.blocks_in_fragmented_chunks_size
            .store(0, Ordering::SeqCst);
    }
}
