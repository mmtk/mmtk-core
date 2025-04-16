use std::marker::PhantomData;

use crate::util::ObjectReference;
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
    remset: Vec<RemSetEntry>,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> EvacuateMatureObjects<VM> {
    pub const CAPACITY: usize = 1024;

    pub(super) fn new(remset: Vec<RemSetEntry>) -> Self {
        debug_assert!(crate::args::RC_MATURE_EVACUATION);
        Self {
            remset,
            _p: PhantomData,
        }
    }

    fn address_is_valid_oop_slot(&self, s: VM::VMSlot, lxr: &LXR<VM>) -> bool {
        // Keep slots not in the mmtk heap
        // These should be slots in the c++ `ClassLoaderData` objects. We remember these slots
        // in the remembered-set to avoid expensive CLD scanning.
        if !lxr.immix_space.address_in_space(s.to_address())
            && !lxr.los().address_in_space(s.to_address())
        {
            return true;
        }
        // Skip slots in collection set
        if lxr.address_in_defrag(s.to_address()) {
            return false;
        }
        if crate::args::NO_RC_PAUSES_DURING_CONCURRENT_MARKING {
            return true;
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

    fn process_slot(&mut self, s: VM::VMSlot, old_ref: ObjectReference, lxr: &LXR<VM>) -> bool {
        // Skip slots that does not contain a real oop
        if !self.address_is_valid_oop_slot(s, lxr) {
            return false;
        }
        // Skip objects that are dead or out of the collection set.
        let Some(o) = s.load() else {
            return false;
        };
        if old_ref != o {
            return false;
        }
        if !o.is_in_any_space() || !lxr.immix_space.in_space(o) {
            return false;
        }
        if !lxr.rc.is_dead(o) && Block::in_defrag_block::<VM>(o) {
            return true;
        }
        false
        // Maybe a forwarded nursery or mature object from inc processing.
        // if object_forwarding::is_forwarded_or_being_forwarded::<VM>(o) {
        //     return true;
        // }
        // rc::count(o) != 0 && Block::in_defrag_block::<VM>(o)
    }

    fn process_slots(&mut self, mmtk: &'static MMTK<VM>) -> Option<Box<dyn GCWork<VM>>> {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        assert_eq!(lxr.current_pause(), Some(Pause::FinalMark));
        let remset = std::mem::take(&mut self.remset);
        let mut slots = vec![];
        let mut refs = vec![];
        for entry in remset {
            let (s, o) = entry.decode::<VM>();
            if self.process_slot(s, o, lxr) {
                slots.push(s);
                refs.push(o);
            }
        }
        if !slots.is_empty() {
            Some(Box::new(
                LXRStopTheWorldProcessEdges::<_, false>::new_remset(slots, refs, mmtk),
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
