use super::{
    block::{Block, BlockState},
    line::Line,
    ImmixSpace,
};
use crate::util::linear_scan::Region;
use crate::{policy::space::Space, Plan};
use crate::{util::constants::LOG_BYTES_IN_PAGE, vm::*};
use spin::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub type Histogram = [usize; Defrag::NUM_BINS];

#[derive(Debug, Default)]
pub struct Defrag {
    /// Is current GC a defrag GC?
    in_defrag_collection: AtomicBool,
    /// Is defrag space exhausted?
    defrag_space_exhausted: AtomicBool,
    /// A list of completed mark histograms reported by workers
    pub mark_histograms: Mutex<Vec<Histogram>>,
    /// A block with number of holes greater than this threshold will be defragmented.
    pub defrag_spill_threshold: AtomicUsize,
    /// The number of remaining clean pages in defrag space.
    available_clean_pages_for_defrag: AtomicUsize,
}

pub struct StatsForDefrag {
    total_pages: usize,
    reserved_pages: usize,
    collection_reserved_pages: usize,
}

impl StatsForDefrag {
    pub fn new<VM: VMBinding>(plan: &dyn Plan<VM = VM>) -> Self {
        Self {
            total_pages: plan.get_total_pages(),
            reserved_pages: plan.get_reserved_pages(),
            collection_reserved_pages: plan.get_collection_reserved_pages(),
        }
    }
}

impl Defrag {
    const NUM_BINS: usize = (Block::LINES >> 1) + 1;
    const DEFRAG_LINE_REUSE_RATIO: f32 = 0.99;
    const MIN_SPILL_THRESHOLD: usize = 2;
    const DEFRAG_HEADROOM_PERCENT: usize = 2;

    /// Allocate a new local histogram.
    pub const fn new_histogram(&self) -> Histogram {
        [0; Self::NUM_BINS]
    }

    /// Report back a completed mark histogram
    pub fn add_completed_mark_histogram(&self, histogram: Histogram) {
        self.mark_histograms.lock().push(histogram)
    }

    /// Check if the current GC is a defrag GC.
    pub fn in_defrag(&self) -> bool {
        self.in_defrag_collection.load(Ordering::Acquire)
    }

    /// Determine whether the current GC should do defragmentation.
    pub fn decide_whether_to_defrag(
        &self,
        emergency_collection: bool,
        collect_whole_heap: bool,
        collection_attempts: usize,
        user_triggered: bool,
        exhausted_reusable_space: bool,
        full_heap_system_gc: bool,
    ) {
        let in_defrag = super::DEFRAG
            && (emergency_collection
                || (collection_attempts > 1)
                || !exhausted_reusable_space
                || super::STRESS_DEFRAG
                || (collect_whole_heap && user_triggered && full_heap_system_gc));
        // println!("Defrag: {}", in_defrag);
        self.in_defrag_collection
            .store(in_defrag, Ordering::Release)
    }

    /// Get the number of defrag headroom pages.
    pub fn defrag_headroom_pages<VM: VMBinding>(&self, space: &ImmixSpace<VM>) -> usize {
        space.get_page_resource().reserved_pages() * Self::DEFRAG_HEADROOM_PERCENT / 100
    }

    /// Check if the defrag space is exhausted.
    pub fn space_exhausted(&self) -> bool {
        self.defrag_space_exhausted.load(Ordering::Acquire)
    }

    /// Update available_clean_pages_for_defrag counter when a clean block is allocated.
    pub fn notify_new_clean_block(&self, copy: bool) {
        if copy {
            let available_clean_pages_for_defrag =
                self.available_clean_pages_for_defrag.fetch_update(
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                    |available_clean_pages_for_defrag| {
                        if available_clean_pages_for_defrag <= Block::PAGES {
                            Some(0)
                        } else {
                            Some(available_clean_pages_for_defrag - Block::PAGES)
                        }
                    },
                );
            if available_clean_pages_for_defrag.unwrap() <= Block::PAGES {
                self.defrag_space_exhausted.store(true, Ordering::SeqCst);
            }
        }
    }

    /// Prepare work. Should be called in ImmixSpace::prepare.
    #[allow(clippy::assertions_on_constants)]
    pub fn prepare<VM: VMBinding>(&self, space: &ImmixSpace<VM>, plan_stats: StatsForDefrag) {
        debug_assert!(super::DEFRAG);
        self.defrag_space_exhausted.store(false, Ordering::Release);

        // Calculate available free space for defragmentation.

        let mut available_clean_pages_for_defrag = plan_stats.total_pages as isize
            - plan_stats.reserved_pages as isize
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
            available_clean_pages_for_defrag as usize + plan_stats.collection_reserved_pages,
            Ordering::Release,
        );
    }

    /// Get the numebr of all the recyclable lines in all the reusable blocks.
    fn get_available_lines<VM: VMBinding>(
        &self,
        space: &ImmixSpace<VM>,
        spill_avail_histograms: &mut Histogram,
    ) -> usize {
        let mut total_available_lines = 0;
        space.reusable_blocks.iterate_blocks(|block| {
            let bucket = block.get_holes();
            let unavailable_lines = match block.get_state() {
                BlockState::Reusable { unavailable_lines } => unavailable_lines as usize,
                s => unreachable!("{:?} {:?}", block, s),
            };
            let available_lines = Block::LINES - unavailable_lines;
            spill_avail_histograms[bucket] += available_lines;
            total_available_lines += available_lines;
        });
        total_available_lines
    }

    /// Calculate the defrag threshold.
    fn establish_defrag_spill_threshold<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        let mut spill_avail_histograms = self.new_histogram();
        let clean_lines = self.get_available_lines(space, &mut spill_avail_histograms);
        let available_lines = clean_lines
            + (self
                .available_clean_pages_for_defrag
                .load(Ordering::Acquire)
                << (LOG_BYTES_IN_PAGE as usize - Line::LOG_BYTES));

        // Number of lines we will evacuate.
        let mut required_lines = 0isize;
        // Number of to-space free lines we can use for defragmentation.
        let mut limit = (available_lines as f32 / Self::DEFRAG_LINE_REUSE_RATIO) as isize;
        let mut threshold = Block::LINES >> 1;
        let mark_histograms = self.mark_histograms.lock();
        // Blocks are grouped by buckets, indexed by the number of holes in the block.
        // `mark_histograms` remembers the number of live lines for each bucket.
        // Here, reversely iterate all the bucket to find a threshold that all buckets above this
        // threshold can be evacuated, without causing to-space overflow.
        for index in (Self::MIN_SPILL_THRESHOLD..Self::NUM_BINS).rev() {
            threshold = index;
            // Calculate total number of live lines in this bucket.
            let this_bucket_mark = mark_histograms
                .iter()
                .map(|v| v[threshold] as isize)
                .sum::<isize>();
            // Calculate the number of free lines in this bucket.
            let this_bucket_avail = spill_avail_histograms[threshold] as isize;
            // Update counters
            limit -= this_bucket_avail;
            required_lines += this_bucket_mark;
            // Stop scanning. Lines to evacuate exceeds the free to-space lines.
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
        debug_assert!(super::DEFRAG);
        self.in_defrag_collection.store(false, Ordering::Release);
    }
}
