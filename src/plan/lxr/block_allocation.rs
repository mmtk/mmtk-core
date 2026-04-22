use super::block_sweeping::{RCLazySweepNurseryBlocks, RCSTWSweepNurseryBlocks};
use super::LXR;
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::policy::immix::block::{Block, BlockState};
use crate::policy::immix::{ImmixHooks, ImmixSpace};
use crate::scheduler::{GCWork, GCWorkScheduler, WorkBucketStage};
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::vm::VMBinding;
use atomic::{Atomic, Ordering};
use std::cell::UnsafeCell;
use std::sync::atomic::AtomicUsize;
use std::sync::RwLock;

struct BlockCache {
    cursor: AtomicUsize,
    buffer: RwLock<Vec<Atomic<Block>>>,
}

impl BlockCache {
    fn new() -> Self {
        Self {
            cursor: AtomicUsize::new(0),
            buffer: RwLock::new((0..32768).map(|_| Atomic::new(Block::ZERO)).collect()),
        }
    }

    fn len(&self) -> usize {
        self.cursor.load(Ordering::SeqCst)
    }

    fn push(&self, block: Block) {
        let i = self.cursor.fetch_add(1, Ordering::SeqCst);
        let buffer = self.buffer.read().unwrap();
        if i < buffer.len() {
            buffer[i].store(block, Ordering::SeqCst);
        } else {
            std::mem::drop(buffer);
            let mut buffer = self.buffer.write().unwrap();
            if i >= buffer.len() {
                buffer.resize_with(i << 1, || Atomic::new(Block::ZERO));
            }
            buffer[i].store(block, Ordering::Relaxed);
        }
    }

    fn visit_slice(&self, f: impl Fn(&[Atomic<Block>])) {
        let count = self.cursor.load(Ordering::SeqCst);
        let blocks = self.buffer.read().unwrap();
        f(&blocks[0..count])
    }

    fn reset(&self) {
        self.cursor.store(0, Ordering::SeqCst);
    }
}

pub(super) struct BlockAllocation<VM: VMBinding> {
    space: UnsafeCell<*const ImmixSpace<VM>>,
    lxr: UnsafeCell<*const LXR<VM>>,
    nursery_blocks: BlockCache,
    reused_blocks: BlockCache,
}

unsafe impl<VM: VMBinding> Sync for BlockAllocation<VM> {}
unsafe impl<VM: VMBinding> Send for BlockAllocation<VM> {}

impl<VM: VMBinding> BlockAllocation<VM> {
    pub(super) fn new() -> Self {
        Self {
            space: UnsafeCell::new(std::ptr::null()),
            lxr: UnsafeCell::new(std::ptr::null()),
            nursery_blocks: BlockCache::new(),
            reused_blocks: BlockCache::new(),
        }
    }

    pub(super) fn init(&self, space: &ImmixSpace<VM>, lxr: &'static LXR<VM>) {
        unsafe {
            *self.space.get() = space as *const ImmixSpace<VM>;
            *self.lxr.get() = lxr as *const LXR<VM>;
        }
    }

    fn space(&self) -> &'static ImmixSpace<VM> {
        unsafe { &**self.space.get() }
    }

    fn lxr(&self) -> &'static LXR<VM> {
        unsafe { &**self.lxr.get() }
    }

    pub(super) fn clean_nursery_mb(&self) -> usize {
        self.nursery_blocks.len() << Block::LOG_BYTES >> 20
    }

    pub(super) fn total_young_allocation_in_bytes(&self) -> usize {
        (self.nursery_blocks.len() << Block::LOG_BYTES)
            + (self.space().get_mutator_recycled_lines_in_pages() << LOG_BYTES_IN_PAGE)
    }

    pub(super) fn reset_block_mark_for_mutator_reused_blocks(&self, _pause: Pause) {
        // SATB sweep has problem scanning mutator recycled blocks.
        // Remaing the block state as "reusing" and reset them here.
        self.reused_blocks.visit_slice(|blocks| {
            for b in blocks {
                let b = b.load(Ordering::Relaxed);
                b.set_state(BlockState::Marked);
            }
        });
    }

    pub(super) fn sweep_mutator_reused_blocks(&self, pause: Pause) {
        if pause == Pause::Full || pause == Pause::FinalMark {
            self.reused_blocks.reset();
            return;
        }
        self.reused_blocks.visit_slice(|blocks| {
            for b in blocks {
                let block = b.load(Ordering::Relaxed);
                self.space()
                    .add_to_possibly_dead_mature_blocks(block, false);
            }
        });
        self.reused_blocks.reset();
    }

    /// Reset allocated_block_buffer and free nursery blocks.
    pub(super) fn sweep_nursery_blocks(&self, scheduler: &GCWorkScheduler<VM>, pause: Pause) {
        const PARALLEL_STW_SWEEPING: bool = false;
        let max_stw_sweep_blocks: usize = usize::MAX;
        let space = self.space();
        self.nursery_blocks.visit_slice(|blocks| {
            if PARALLEL_STW_SWEEPING {
                return self.parallel_sweep_all_nursery_blocks(scheduler, blocks);
            }
            let total_nursery_blocks = blocks.len();
            let stw_limit = if pause == Pause::Full {
                total_nursery_blocks
            } else {
                usize::min(total_nursery_blocks, max_stw_sweep_blocks)
            };
            for b in &blocks[0..stw_limit] {
                let block = b.load(Ordering::Relaxed);
                debug_assert_ne!(block.get_state(), BlockState::Unallocated);
                block.rc_sweep_nursery(space);
            }
            if total_nursery_blocks > stw_limit {
                let packets = blocks[stw_limit..total_nursery_blocks]
                    .chunks(1024)
                    .map(|c| {
                        let blocks: Vec<Block> =
                            c.iter().map(|x| x.load(Ordering::Relaxed)).collect();
                        Box::new(RCLazySweepNurseryBlocks::new(blocks)) as Box<dyn GCWork<VM>>
                    })
                    .collect();
                scheduler.work_buckets[WorkBucketStage::Concurrent].bulk_add_deferred(packets);
            }
        });
        self.nursery_blocks.reset();
    }

    fn parallel_sweep_all_nursery_blocks(
        &self,
        scheduler: &GCWorkScheduler<VM>,
        blocks: &[Atomic<Block>],
    ) {
        let total_nursery_blocks = blocks.len();
        let packets = blocks[..total_nursery_blocks]
            .chunks(1024)
            .map(|c| {
                let blocks: Vec<Block> = c.iter().map(|x| x.load(Ordering::Relaxed)).collect();
                Box::new(RCSTWSweepNurseryBlocks::new(blocks)) as Box<dyn GCWork<VM>>
            })
            .collect();
        scheduler.work_buckets[WorkBucketStage::Unconstrained].bulk_add(packets);
    }
}

impl<VM: VMBinding> ImmixHooks<VM> for BlockAllocation<VM> {
    fn on_clean_block_acquired(&self, block: Block, copy: bool) {
        if !copy {
            self.nursery_blocks.push(block);
        }
        if copy {
            block.initialize_field_unlog_table_as_unlogged::<VM>();
        }
        if self.cm_in_progress_or_final_mark() {
            block.initialize_mark_table_as_marked::<VM>();
        } else {
            block.clear_mark_table::<VM>();
        }
    }

    fn on_reusable_block_acquired(&self, block: Block, copy: bool) {
        if !copy {
            self.reused_blocks.push(block);
        }
    }

    fn cm_in_progress_or_final_mark(&self) -> bool {
        let lxr = self.lxr();
        lxr.concurrent_work_in_progress() || lxr.current_pause() == Some(Pause::FinalMark)
    }
}
