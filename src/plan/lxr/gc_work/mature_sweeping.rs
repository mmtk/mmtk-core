use std::ops::Range;
use std::sync::atomic::Ordering;

use crate::plan::lxr::{LazySweepingJobsCounter, LXR};
use crate::policy::immix::block::{Block, BlockState};
use crate::policy::immix::line::Line;
use crate::policy::immix::ImmixSpace;
use crate::scheduler::{GCWork, GCWorker};
use crate::util::heap::chunk_map::Chunk;
use crate::util::linear_scan::Region;
use crate::util::rc::{self, RefCountHelper};
use crate::util::ObjectReference;
use crate::vm::VMBinding;
use crate::MMTK;

/// Chunk sweeping work packet.
pub struct SweepDeadCycles<VM: VMBinding> {
    chunks: Range<Chunk>,
    _counter: LazySweepingJobsCounter,
    rc: RefCountHelper<VM>,
}

#[allow(unused)]
impl<VM: VMBinding> SweepDeadCycles<VM> {
    const CAPACITY: usize = 1024;

    pub fn new(chunks: Range<Chunk>, counter: LazySweepingJobsCounter) -> Self {
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

    fn process_block(&mut self, block: Block, lxr: &LXR<VM>, immix_space: &ImmixSpace<VM>) {
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
            lxr.add_to_possibly_dead_mature_blocks(block, false);
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
                    self.process_block(block, lxr, immix_space)
                }
            }
        }
    }
}

pub struct RCSweepMatureAfterSATBLOS {
    _counter: LazySweepingJobsCounter,
}

impl RCSweepMatureAfterSATBLOS {
    pub fn new(counter: LazySweepingJobsCounter) -> Self {
        Self { _counter: counter }
    }
}

impl<VM: VMBinding> GCWork<VM> for RCSweepMatureAfterSATBLOS {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let los = mmtk.get_plan().common().get_los();
        los.sweep_rc_mature_objects_after_satb(&|o| !(!los.is_marked(o) && los.rc.count(o) != 0));
    }
}
