use atomic::Ordering;

use crate::plan::lxr::{LazySweepingJobsCounter, LXR};
use crate::policy::immix::block::Block;
use crate::scheduler::WorkBucketStage;
use crate::scheduler::{GCWork, GCWorker};
use crate::vm::VMBinding;
use crate::MMTK;

pub(crate) struct RCLazySweepNurseryBlocks {
    blocks: Vec<Block>,
    _counter: LazySweepingJobsCounter,
}

impl RCLazySweepNurseryBlocks {
    pub(crate) fn new(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            _counter: LazySweepingJobsCounter::new_decs(),
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for RCLazySweepNurseryBlocks {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = &mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        let mut released_blocks = 0;
        for block in &self.blocks {
            if block.rc_sweep_nursery(&lxr.immix_space) {
                released_blocks += 1;
            }
        }
        lxr.num_clean_blocks_released_lazy
            .fetch_add(released_blocks, Ordering::SeqCst);
    }
}

pub(crate) struct RCSTWSweepNurseryBlocks {
    blocks: Vec<Block>,
    _counter: LazySweepingJobsCounter,
}

impl RCSTWSweepNurseryBlocks {
    pub(crate) fn new(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            _counter: LazySweepingJobsCounter::new_decs(),
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for RCSTWSweepNurseryBlocks {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        for block in &self.blocks {
            block.rc_sweep_nursery(space);
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
            lxr.num_clean_blocks_released_lazy
                .fetch_add(count, Ordering::Relaxed);
        }
    }
}

pub struct ReleaseLOSNursery;

impl<VM: VMBinding> GCWork<VM> for ReleaseLOSNursery {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        lxr.los().release_rc_nursery_objects();
    }
}
