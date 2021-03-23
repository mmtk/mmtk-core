use std::{ops::Range, sync::atomic::{AtomicBool, AtomicUsize, Ordering}};
use crate::{MMTK, scheduler::{GCWork, GCWorker, GCWorkBucket, WorkBucketStage}, util::constants::LOG_BYTES_IN_PAGE, vm::*};
use crate::policy::space::Space;
use super::{ImmixSpace, block::{Block, BlockState}, chunk::{Chunk, ChunkState}, line::Line};


#[derive(Debug, Default)]
pub struct Defrag {
    in_defrag_collection: AtomicBool,
    defrag_space_exhausted: AtomicBool,
    pub spill_mark_histograms: Vec<Vec<AtomicUsize>>,
    spill_avail_histograms: Vec<AtomicUsize>,
    pub defrag_spill_threshold: AtomicUsize,
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
            ..Default::default()
        }
    }

    pub fn prepare_histograms<VM: VMBinding>(&self, mmtk: &MMTK<VM>) {
        let self_mut = unsafe { &mut *(self as *const _ as *mut Self) };
        self_mut.spill_mark_histograms.resize_with(mmtk.options.threads, || (0..Self::NUM_BINS).map(|_| Default::default()).collect());
    }

    #[inline(always)]
    pub fn in_defrag(&self) -> bool {
        self.in_defrag_collection.load(Ordering::SeqCst)
    }

    pub fn decide_whether_to_defrag(&self, emergency_collection: bool, collection_attempts: usize, exhausted_reusable_space: bool) {
        let in_defrag = super::DEFRAG && (emergency_collection || (collection_attempts > 1) || !exhausted_reusable_space || Self::DEFRAG_STRESS);
        println!("Defrag: {}", in_defrag);
        self.in_defrag_collection.store(in_defrag, Ordering::SeqCst)
    }

    pub fn defrag_headroom_pages<VM: VMBinding>(&self, space: &ImmixSpace<VM>) -> usize {
        space.get_page_resource().reserved_pages() * 2 / 100
    }

    pub fn prepare<VM: VMBinding>(&'static self, space: &'static ImmixSpace<VM>) {
        debug_assert!(!super::BLOCK_ONLY);
        let mut available_clean_pages_for_defrag = VM::VMActivePlan::global().get_total_pages() as isize - VM::VMActivePlan::global().get_pages_reserved() as isize + self.defrag_headroom_pages(space) as isize;
        if available_clean_pages_for_defrag < 0 { available_clean_pages_for_defrag = 0 };

        self.available_clean_pages_for_defrag.store(available_clean_pages_for_defrag as usize, Ordering::Release);

        if self.in_defrag() {
            self.establish_defrag_spill_threshold(space)
        }

        self.available_clean_pages_for_defrag.store(available_clean_pages_for_defrag as usize + VM::VMActivePlan::global().get_collection_reserve(), Ordering::Release);
    }

    fn get_available_lines<VM: VMBinding>(&self, space: &ImmixSpace<VM>) -> usize {
        for entry in &self.spill_avail_histograms {
            entry.store(0, Ordering::Relaxed);
        }
        let mut total_available_lines = 0;
        for block in &space.reusable_blocks {
            let bucket = block.get_holes();
            let unavailable_lines = match block.get_state() {
                BlockState::Reusable { unavailable_lines } => unavailable_lines as usize,
                _ => unreachable!(),
            };
            let available_lines = Block::LINES - unavailable_lines;
            let old = self.spill_avail_histograms[bucket].load(Ordering::Relaxed);
            self.spill_avail_histograms[bucket].store(old + available_lines, Ordering::Relaxed);
            total_available_lines += available_lines;
        }
        total_available_lines
    }

    fn establish_defrag_spill_threshold<VM: VMBinding>(&self, space: &ImmixSpace<VM>) {
        let clean_lines = self.get_available_lines(space);
        let available_lines = clean_lines + (self.available_clean_pages_for_defrag.load(Ordering::Acquire) << (LOG_BYTES_IN_PAGE as usize - Line::LOG_BYTES));

        let mut required_lines = 0isize;
        let mut limit = (available_lines as f32 / Self::DEFRAG_LINE_REUSE_RATIO) as isize;
        let mut threshold = Block::LINES >> 1;
        for index in (Self::MIN_SPILL_THRESHOLD..Self::NUM_BINS).rev() {
            threshold = index;
            let this_bucket_mark = self.spill_mark_histograms.iter().map(|v| v[threshold].load(Ordering::Acquire) as isize).sum::<isize>();
            let this_bucket_avail = self.spill_avail_histograms[threshold].load(Ordering::Acquire) as isize;
            limit -= this_bucket_avail as isize;
            required_lines += this_bucket_mark;
            if limit < required_lines {
                break
            }
        }
        println!("threshold: {}", threshold);
        debug_assert!(threshold >= Self::MIN_SPILL_THRESHOLD);
        self.defrag_spill_threshold.store(threshold, Ordering::Release);
    }

    pub fn release<VM: VMBinding>(&self, _space: &ImmixSpace<VM>) {
        debug_assert!(!super::BLOCK_ONLY);
        self.in_defrag_collection.store(false, Ordering::SeqCst);
    }
}
