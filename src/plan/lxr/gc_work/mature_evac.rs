use std::marker::PhantomData;
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::tracing::LXRStopTheWorldProcessEdges;
use crate::plan::lxr::mature_evac::MatureEvacuationSet;
use crate::policy::immix::line::Line;
use crate::util::heap::chunk_map::Chunk;
use crate::util::linear_scan::Region;
use crate::util::metadata::side_metadata::spec_defs::{IX_LINE_REUSE_COUNT, LOS_PAGE_REUSE_COUNT};
use crate::vm::slot::Slot;
use crate::{
    plan::concurrent::Pause,
    policy::{
        immix::block::{Block, BlockState},
        space::Space,
    },
    scheduler::{GCWork, GCWorker, WorkBucketStage},
    vm::VMBinding,
    MMTK,
};

use super::super::mature_evac::RemSetEntry;
use super::super::LXR;

pub static SELECT_DEFRAG_BLOCK_JOB_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct SelectDefragBlocks {
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
            lxr.evac_set
                .fragmented_blocks_size
                .fetch_add(fragmented_blocks.len(), Ordering::SeqCst);
            lxr.evac_set.fragmented_blocks.push(fragmented_blocks);
        }

        if SELECT_DEFRAG_BLOCK_JOB_COUNTER.fetch_sub(1, Ordering::SeqCst) == 1 {
            lxr.evac_set.select_mature_evacuation_candidates(lxr)
        }
    }
}

pub struct EvacuateMatureObjects<VM: VMBinding> {
    remset: Vec<RemSetEntry<VM>>,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> EvacuateMatureObjects<VM> {
    pub const CAPACITY: usize = 1024;

    pub fn new(remset: Vec<RemSetEntry<VM>>) -> Self {
        debug_assert!(super::super::MATURE_EVACUATION);
        Self {
            remset,
            _p: PhantomData,
        }
    }

    fn address_is_valid_oop_slot(&self, s: VM::VMSlot, original_reuse: u8, lxr: &LXR<VM>) -> bool {
        // Keep slots not in the mmtk heap
        // These should be slots in the c++ `ClassLoaderData` objects. We remember these slots
        // in the remembered-set to avoid expensive CLD scanning.
        let addr = s.to_address();
        // Check reuse count
        if lxr.immix_space.address_in_space(addr) {
            let reuse = IX_LINE_REUSE_COUNT.load_atomic::<u8>(addr, atomic::Ordering::SeqCst);
            if reuse != original_reuse {
                return false;
            }
        } else if lxr.los().address_in_space(addr) {
            let reuse = LOS_PAGE_REUSE_COUNT.load_atomic::<u8>(addr, atomic::Ordering::SeqCst);
            if reuse != original_reuse {
                return false;
            }
        } else {
            return false;
        }
        // Skip slots in collection set
        if lxr.address_in_defrag(addr) {
            return false;
        }
        // Check if it is a real oop field
        if lxr.immix_space.address_in_space(s.to_address()) {
            let block = Block::of(s.to_address());
            if block.get_state() == BlockState::Unallocated {
                return false;
            }
        }
        true
    }

    fn process_slot(&mut self, s: VM::VMSlot, reuse: u8, lxr: &LXR<VM>) -> bool {
        // Skip slots that does not contain a real oop
        if !self.address_is_valid_oop_slot(s, reuse, lxr) {
            return false;
        }
        // Skip objects that are dead or out of the collection set.
        let v = unsafe { s.to_address().load::<u32>() };
        if v & 0b111 != 0 {
            panic!("Invalid slot: {s:?} -> {v:#x}");
        }
        let Some(o) = s.load() else {
            return false;
        };
        if !o.is_in_any_space() || !lxr.immix_space.in_space(o) {
            return false;
        }
        if !lxr.rc.is_dead(o) && Block::in_defrag_block::<VM>(o) {
            return true;
        }
        false
    }

    fn process_slots(&mut self, mmtk: &'static MMTK<VM>) -> Option<Box<dyn GCWork<VM>>> {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        assert_eq!(lxr.current_pause(), Some(Pause::FinalMark));
        let remset = std::mem::take(&mut self.remset);
        let mut slots = vec![];
        for entry in remset {
            let (s, reuse) = entry.decode();
            if self.process_slot(s, reuse, lxr) {
                slots.push(s);
            }
        }
        if !slots.is_empty() {
            Some(Box::new(
                LXRStopTheWorldProcessEdges::<_, false>::new_remset(slots, mmtk),
            ))
        } else {
            None
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for EvacuateMatureObjects<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let Some(work) = self.process_slots(mmtk) else {
            return;
        };
        // transitive closure
        worker.add_boxed_work(WorkBucketStage::Closure, work)
    }
}
