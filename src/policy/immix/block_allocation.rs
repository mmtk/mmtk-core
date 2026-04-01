use super::{block::Block, ImmixSpace};
use crate::plan::immix::Pause;
use crate::util::constants::LOG_BYTES_IN_PAGE;
use crate::{plan::lxr::LXR, policy::space::Space, scheduler::GCWorkScheduler, vm::*};
use atomic::Ordering;
use std::cell::UnsafeCell;
use std::sync::atomic::AtomicUsize;

pub struct BlockAllocation<VM: VMBinding> {
    space: UnsafeCell<*const ImmixSpace<VM>>,
    pub(crate) lxr: Option<&'static LXR<VM>>,
    num_nursery_blocks: AtomicUsize,
    pub(crate) in_place_promoted_nursery_blocks: AtomicUsize,
}

impl<VM: VMBinding> BlockAllocation<VM> {
    pub fn new() -> Self {
        Self {
            space: UnsafeCell::new(std::ptr::null()),
            lxr: None,
            num_nursery_blocks: AtomicUsize::new(0),
            in_place_promoted_nursery_blocks: Default::default(),
        }
    }

    fn space(&self) -> &'static ImmixSpace<VM> {
        unsafe { &**self.space.get() }
    }

    pub fn clean_nursery_blocks(&self) -> usize {
        self.num_nursery_blocks.load(Ordering::Relaxed)
    }

    pub fn clean_nursery_mb(&self) -> usize {
        self.clean_nursery_blocks() << Block::LOG_BYTES >> 20
    }

    pub fn total_young_allocation_in_bytes(&self) -> usize {
        (self.clean_nursery_blocks() << Block::LOG_BYTES)
            + (self.space().get_mutator_recycled_lines_in_pages() << LOG_BYTES_IN_PAGE)
    }

    pub fn init(&self, space: &ImmixSpace<VM>) {
        unsafe { *self.space.get() = space as *const ImmixSpace<VM> }
    }

    /// Reset allocated_block_buffer and free nursery blocks.
    pub fn sweep_nursery_blocks(&self, _scheduler: &GCWorkScheduler<VM>, _pause: Pause) {
        let in_place_promoted_nursery_blocks = self
            .in_place_promoted_nursery_blocks
            .load(Ordering::Relaxed);
        let num_blocks = self.clean_nursery_blocks();
        self.space()
            .pr
            .bulk_release_blocks(num_blocks - in_place_promoted_nursery_blocks);
        self.space().pr.reset();
        self.num_nursery_blocks.store(0, Ordering::SeqCst);
        self.in_place_promoted_nursery_blocks
            .store(0, Ordering::SeqCst);
    }

    /// Notify a GC pahse has started
    pub fn notify_mutator_phase_end(&self) {}

    pub fn cm_in_progress_or_final_mark(&self) -> bool {
        let lxr = self.lxr.unwrap();
        lxr.cm_in_progress() || lxr.current_pause() == Some(Pause::FinalMark)
    }

    pub(super) fn initialize_new_clean_block(&self, block: Block, copy: bool, cm_enabled: bool) {
        if self.space().in_defrag() {
            self.space().defrag.notify_new_clean_block(copy);
        }
        if cm_enabled && !super::BLOCK_ONLY && !self.space().rc_enabled {
            let current_state = self.space().line_mark_state.load(Ordering::Acquire);
            for line in block.lines() {
                line.mark(current_state);
            }
        }
        // Initialize unlog table
        if (self.space().rc_enabled || crate::plan::barriers::BARRIER_MEASUREMENT) && copy {
            block.initialize_field_unlog_table_as_unlogged::<VM>();
        }
        // Initialize mark table
        if self.space().rc_enabled {
            if self.cm_in_progress_or_final_mark() {
                block.initialize_mark_table_as_marked::<VM>();
            } else {
                // TODO: Performance? Is this necessary?
                block.clear_mark_table::<VM>();
            }
            if !copy {
                self.num_nursery_blocks.fetch_add(1, Ordering::Relaxed);
                block.clear_field_unlog_table::<VM>();
            }
        }
        // println!("Alloc {:?} {}", block, copy);
        block.init(copy, false, self.space());
        if self.space().common().zeroed && !copy && cfg!(feature = "force_zeroing") {
            crate::util::memory::zero(block.start(), Block::BYTES);
        }
    }
}
