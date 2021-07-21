use super::{
    block::{Block, BlockState},
    line::Line,
    ImmixSpace,
};
use crate::policy::space::Space;
use crate::{util::constants::LOG_BYTES_IN_PAGE, vm::*, MMTK};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, RwLock,
};

#[derive(Debug, Default)]
pub struct Defrag {
    /// Is current GC a defrag GC?
    in_defrag_collection: AtomicBool,
    /// Is defrag space exhausted?
    defrag_space_exhausted: AtomicBool,
    /// per-worker mark histograms
    pub spill_mark_histograms: RwLock<Vec<Arc<Vec<AtomicUsize>>>>,
    spill_avail_histograms: Vec<AtomicUsize>,
    pub defrag_spill_threshold: AtomicUsize,
    /// The number of remaining clean pages in defrag space.
    available_clean_pages_for_defrag: AtomicUsize,
}

impl Defrag {
    const NUM_BINS: usize = (Block::LINES >> 1) + 1;
    const DEFRAG_LINE_REUSE_RATIO: f32 = 0.99;
    const MIN_SPILL_THRESHOLD: usize = 2;
    const DEFRAG_STRESS: bool = false;

    pub fn new() -> Self {
        Self {
            spill_avail_histograms: (0..Self::NUM_BINS).map(|_| Default::default()).collect(),
            spill_mark_histograms: Default::default(),
            ..Default::default()
        }
    }

    /// Get worker-local mark histogram
    pub fn mark_histogram(&self, index: usize) -> Arc<Vec<AtomicUsize>> {
        self.spill_mark_histograms.read().unwrap()[index].clone()
    }

    /// Prepare spill mark histograms
    pub fn prepare_histograms<VM: VMBinding>(&self, mmtk: &MMTK<VM>) {
        self.spill_mark_histograms
            .write()
            .unwrap()
            .resize_with(mmtk.options.threads, || {
                Arc::new((0..Self::NUM_BINS).map(|_| Default::default()).collect())
            });
    }

    /// Check if the current GC is a defrag GC.
    #[inline(always)]
    pub fn in_defrag(&self) -> bool {
        self.in_defrag_collection.load(Ordering::Acquire)
    }

    /// Determine whether the current GC should do defragmentation.
    pub fn decide_whether_to_defrag(
        &self,
        emergency_collection: bool,
        collection_attempts: usize,
        exhausted_reusable_space: bool,
    ) {
        let in_defrag = super::DEFRAG
            && (emergency_collection
                || (collection_attempts > 1)
                || !exhausted_reusable_space
                || Self::DEFRAG_STRESS);
        // println!("Defrag: {}", in_defrag);
        self.in_defrag_collection
            .store(in_defrag, Ordering::Release)
    }

    /// Get the number of defrag headroom pages.
    pub fn defrag_headroom_pages<VM: VMBinding>(&self, space: &ImmixSpace<VM>) -> usize {
        space.get_page_resource().reserved_pages() * 2 / 100
    }

    /// Check if the defrag space is exhausted.
    #[inline(always)]
    pub fn space_exhausted(&self) -> bool {
        self.defrag_space_exhausted.load(Ordering::Acquire)
    }

    /// Update available_clean_pages_for_defrag counter when a clean block is allocated.
    pub fn notify_new_clean_block(&self, copy: bool) {
        if copy {
            let available_clean_pages_for_defrag = self
                .available_clean_pages_for_defrag
                .load(Ordering::Acquire);
            if available_clean_pages_for_defrag <= Block::PAGES {
                self.available_clean_pages_for_defrag
                    .store(0, Ordering::Release);
                self.defrag_space_exhausted.store(true, Ordering::Release);
            } else {
                self.available_clean_pages_for_defrag.store(
                    available_clean_pages_for_defrag - Block::PAGES,
                    Ordering::Release,
                );
            }
        }
    }

    /// Release work. Should be called in ImmixSpace::prepare.
    #[allow(clippy::assertions_on_constants)]
    pub fn prepare<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        debug_assert!(!super::BLOCK_ONLY);
        self.defrag_space_exhausted.store(false, Ordering::Release);

        // Calculate available free space for defragmentation.

        let mut available_clean_pages_for_defrag = VM::VMActivePlan::global().get_total_pages()
            as isize
            - VM::VMActivePlan::global().get_pages_reserved() as isize
            + self.defrag_headroom_pages(space) as isize;
        if available_clean_pages_for_defrag < 0 {
            available_clean_pages_for_defrag = 0
        };

        self.available_clean_pages_for_defrag
            .store(available_clean_pages_for_defrag as usize, Ordering::Release);

        if self.in_defrag() {
            self.establish_defrag_spill_threshold(space)
        }

        self.available_clean_pages_for_defrag.store(
            available_clean_pages_for_defrag as usize
                + VM::VMActivePlan::global().get_collection_reserve(),
            Ordering::Release,
        );
    }

    /// Get the numebr of all the recyclable lines in all the reusable blocks.
    fn get_available_lines<VM: VMBinding>(&self, space: &ImmixSpace<VM>) -> usize {
        for entry in &self.spill_avail_histograms {
            entry.store(0, Ordering::Relaxed);
        }
        let mut total_available_lines = 0;
        for block in &space.reusable_blocks {
            let bucket = block.get_holes();
            let unavailable_lines = match block.get_state() {
                BlockState::Reusable { unavailable_lines } => unavailable_lines as usize,
                s => unreachable!("{:?} {:?}", block, s),
            };
            let available_lines = Block::LINES - unavailable_lines;
            let old = self.spill_avail_histograms[bucket].load(Ordering::Relaxed);
            self.spill_avail_histograms[bucket].store(old + available_lines, Ordering::Relaxed);
            total_available_lines += available_lines;
        }
        total_available_lines
    }

    /// Calculate the defrag threshold.
    fn establish_defrag_spill_threshold<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        let clean_lines = self.get_available_lines(space);
        let available_lines = clean_lines
            + (self
                .available_clean_pages_for_defrag
                .load(Ordering::Acquire)
                << (LOG_BYTES_IN_PAGE as usize - Line::LOG_BYTES));

        let mut required_lines = 0isize;
        let mut limit = (available_lines as f32 / Self::DEFRAG_LINE_REUSE_RATIO) as isize;
        let mut threshold = Block::LINES >> 1;
        let spill_mark_histograms = self.spill_mark_histograms.read().unwrap();
        for index in (Self::MIN_SPILL_THRESHOLD..Self::NUM_BINS).rev() {
            threshold = index;
            let this_bucket_mark = spill_mark_histograms
                .iter()
                .map(|v| v[threshold].load(Ordering::Acquire) as isize)
                .sum::<isize>();
            let this_bucket_avail =
                self.spill_avail_histograms[threshold].load(Ordering::Acquire) as isize;
            limit -= this_bucket_avail as isize;
            required_lines += this_bucket_mark;
            if limit < required_lines {
                break;
            }
        }
        // println!("threshold: {}", threshold);
        debug_assert!(threshold >= Self::MIN_SPILL_THRESHOLD);
        self.defrag_spill_threshold
            .store(threshold, Ordering::Release);
    }

    /// Release work. Should be called in ImmixSpace::release.
    #[allow(clippy::assertions_on_constants)]
    pub fn release<VM: VMBinding>(&self, _space: &ImmixSpace<VM>) {
        debug_assert!(!super::BLOCK_ONLY);
        self.in_defrag_collection.store(false, Ordering::Release);
    }
}
