use std::marker::PhantomData;

use crate::util::metadata::side_metadata::spec_defs::{IX_LINE_REUSE_COUNT, LOS_PAGE_REUSE_COUNT};
use crate::vm::slot::Slot;
use crate::{
    plan::{immix::Pause, lxr::cm::LXRStopTheWorldProcessEdges},
    policy::{
        immix::block::{Block, BlockState},
        space::Space,
    },
    scheduler::{GCWork, GCWorker, WorkBucketStage},
    vm::VMBinding,
    MMTK,
};

use super::remset::RemSetEntry;
use super::LXR;

pub struct EvacuateMatureObjects<VM: VMBinding> {
    remset: Vec<RemSetEntry<VM>>,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> EvacuateMatureObjects<VM> {
    pub const CAPACITY: usize = 1024;

    pub(super) fn new(remset: Vec<RemSetEntry<VM>>) -> Self {
        debug_assert!(super::MATURE_EVACUATION);
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
