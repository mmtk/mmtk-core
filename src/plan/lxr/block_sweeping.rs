use atomic::Ordering;

use crate::plan::lxr::{LazySweepingJobsCounter, LXR};
use crate::policy::immix::block::Block;
use crate::scheduler::{GCWork, GCWorker};
use crate::vm::VMBinding;
use crate::MMTK;

pub(crate) struct RCLazySweepMutatorReusedBlocks {
    blocks: Vec<Block>,
    _counter: LazySweepingJobsCounter,
}

impl RCLazySweepMutatorReusedBlocks {
    pub(crate) fn new(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            _counter: LazySweepingJobsCounter::new_decs(),
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for RCLazySweepMutatorReusedBlocks {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        for block in &self.blocks {
            space.add_to_possibly_dead_mature_blocks(*block, false);
        }
    }
}

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
        let space = &mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .immix_space;
        let mut released_blocks = 0;
        for block in &self.blocks {
            if block.rc_sweep_nursery(space) {
                released_blocks += 1;
            }
        }
        space
            .num_clean_blocks_released_lazy
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
