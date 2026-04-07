//! Read/Write barrier implementations.

use std::sync::Arc;

use atomic::Ordering;

use super::LXR;
use crate::plan::barriers::BarrierSemantics;
use crate::plan::barriers::{LOGGED_VALUE, UNLOGGED_VALUE};
use crate::plan::immix::Pause;
use crate::plan::lxr::cm::ProcessModBufSATB;
use crate::plan::lxr::rc::ProcessDecs;
use crate::plan::lxr::rc::ProcessIncs;
use crate::plan::lxr::rc::EDGE_KIND_MATURE;
use crate::plan::VectorQueue;
use crate::scheduler::WorkBucketStage;
use crate::util::metadata::side_metadata::address_to_meta_address;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::*;
use crate::vm::slot::MemorySlice;
use crate::vm::slot::Slot;
use crate::vm::*;
use crate::LazySweepingJobsCounter;
use crate::MMTK;

pub struct LXRFieldBarrierSemantics<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
    incs: VectorQueue<VM::VMSlot>,
    decs: VectorQueue<ObjectReference>,
    refs: VectorQueue<ObjectReference>,
    lxr: &'static LXR<VM>,
}

impl<VM: VMBinding> LXRFieldBarrierSemantics<VM> {
    const UNLOG_BITS: SideMetadataSpec = *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
        .as_spec()
        .extract_side_spec();

    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            mmtk,
            incs: VectorQueue::default(),
            decs: VectorQueue::default(),
            refs: VectorQueue::default(),
            lxr: mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap(),
        }
    }

    fn get_slot_logging_state(&self, slot: VM::VMSlot) -> u8 {
        unsafe { Self::UNLOG_BITS.load(slot.to_address()) }
    }

    fn attempt_to_log_field(&self, slot: VM::VMSlot) -> bool {
        loop {
            // Bailout if logged
            if self.get_slot_logging_state(slot) == LOGGED_VALUE {
                return false;
            }
            // Attempt to log the slots
            match Self::UNLOG_BITS.compare_exchange_atomic(
                slot.to_address(),
                UNLOGGED_VALUE,
                LOGGED_VALUE,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(current) => {
                    if current == LOGGED_VALUE {
                        return false;
                    }
                }
            }
            // Failed to log the slot. Spin.
            std::hint::spin_loop();
        }
    }

    fn log_slot_and_get_old_target(&self, slot: VM::VMSlot) -> Result<Option<ObjectReference>, ()> {
        if self.get_slot_logging_state(slot) == LOGGED_VALUE {
            return Err(());
        }
        let old = slot.load();
        if self.attempt_to_log_field(slot) {
            Ok(old)
        } else {
            Err(())
        }
    }

    #[allow(unused)]
    fn log_slot_and_get_old_target_sloppy(
        &self,
        slot: VM::VMSlot,
    ) -> Result<Option<ObjectReference>, ()> {
        if !slot.to_address().is_field_logged::<VM>() {
            let old = slot.load();
            slot.to_address().log_field::<VM>();
            Ok(old)
        } else {
            Err(())
        }
    }

    fn slow(
        &mut self,
        _src: Option<ObjectReference>,
        slot: VM::VMSlot,
        old: Option<ObjectReference>,
    ) {
        // Reference counting
        if let Some(old) = old {
            self.decs.push(old);
            if self.decs.is_full() {
                self.flush_decs_and_satb();
            }
        }
        self.incs.push(slot);
        if self.incs.is_full() {
            self.flush_incs();
        }
    }

    fn enqueue_node(
        &mut self,
        src: Option<ObjectReference>,
        slot: VM::VMSlot,
        _new: Option<ObjectReference>,
    ) -> bool {
        if let Ok(old) = self.log_slot_and_get_old_target(slot) {
            self.slow(src, slot, old);
            true
        } else {
            false
        }
    }

    fn should_create_satb_packets(&self) -> bool {
        self.lxr.cm_enabled()
            && (self.lxr.cm_in_progress() || self.lxr.current_pause() == Some(Pause::FinalMark))
    }

    #[cold]
    fn flush_incs(&mut self) {
        if !self.incs.is_empty() {
            let incs = self.incs.take();
            self.lxr.rc.increase_inc_buffer_size(incs.len());
            self.mmtk.scheduler.work_buckets[WorkBucketStage::RCProcessIncs].add(ProcessIncs::<
                _,
                EDGE_KIND_MATURE,
            >::new(
                incs, self.lxr
            ));
        }
    }

    #[cold]
    fn flush_decs_and_satb(&mut self) {
        if !self.decs.is_empty() {
            let w = if self.should_create_satb_packets() {
                let decs = Arc::new(self.decs.take());
                self.mmtk.scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork]
                    .add(ProcessModBufSATB::new_arc(decs.clone()));
                ProcessDecs::new_arc(decs, LazySweepingJobsCounter::new_decs())
            } else {
                let decs = self.decs.take();
                ProcessDecs::new(decs, LazySweepingJobsCounter::new_decs())
            };
            if crate::args::LAZY_DECREMENTS {
                self.mmtk.scheduler.postpone_prioritized(w);
            } else {
                self.mmtk.scheduler.work_buckets[WorkBucketStage::STWRCDecsAndSweep].add(w);
            }
        }
    }

    #[cold]
    fn flush_weak_refs(&mut self) {
        if !self.refs.is_empty() {
            debug_assert!(self.should_create_satb_packets());
            let nodes = self.refs.take();
            self.mmtk.scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork]
                .add(ProcessModBufSATB::new(nodes));
        }
    }
}

impl<VM: VMBinding> BarrierSemantics for LXRFieldBarrierSemantics<VM> {
    type VM = VM;

    #[cold]
    fn flush(&mut self) {
        self.flush_weak_refs();
        self.flush_incs();
        self.flush_decs_and_satb();
    }

    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        slot: VM::VMSlot,
        target: Option<ObjectReference>,
    ) {
        self.enqueue_node(Some(src), slot, target);
    }

    fn memory_region_copy_slow(&mut self, _src: VM::VMMemorySlice, dst: VM::VMMemorySlice) {
        // Quickly check if all fields are logged. If yes, skip the barrier.
        let unlog_bits_start = address_to_meta_address(&Self::UNLOG_BITS, dst.start());
        let unlog_bits_start_aligned = unlog_bits_start.align_down(16);
        let unlog_bits_end =
            address_to_meta_address(&Self::UNLOG_BITS, dst.start() + dst.bytes() - 1);
        let unlog_bits_end_aligned = unlog_bits_end.align_down(16);
        let mut cursor = unlog_bits_start_aligned;
        let mut all_logged = true;
        while cursor <= unlog_bits_end_aligned {
            if unsafe { cursor.load::<u128>() } != 0 {
                all_logged = false;
                break;
            }
            cursor = cursor + 16usize;
        }
        if all_logged {
            return;
        }

        for s in dst.iter_slots() {
            let _succ = self.enqueue_node(ObjectReference::NULL, s, None);
        }
    }

    fn load_weak_reference(&mut self, o: ObjectReference) {
        if !self.lxr.cm_in_progress() || self.lxr.is_marked(o) {
            return;
        }
        self.refs.push(o);
        if self.refs.is_full() {
            self.flush_weak_refs();
        }
    }

    fn object_probable_write_slow(&mut self, obj: ObjectReference) {
        obj.iterate_fields::<VM, _>(|s| {
            let _succ = self.enqueue_node(Some(obj), s, None);
        });
    }
}
