use super::super::LXR;
use crate::policy::immix::block::{Block, BlockState};
use crate::scheduler::{GCWork, GCWorker};
use crate::util::heap::chunk_map::Chunk;
use crate::util::linear_scan::Region;
use crate::{vm::*, Plan, MMTK};
use std::ops::Range;

pub struct FastRCPrepare;

impl<VM: VMBinding> GCWork<VM> for FastRCPrepare {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        #[allow(invalid_reference_casting)]
        let lxr = unsafe { &mut *(lxr as *const LXR<VM> as *mut LXR<VM>) };
        lxr.prepare(worker.tls)
    }
}

pub struct ConcurrentChunkMetadataZeroing {
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
pub struct PrepareChunksForFullGC {
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
