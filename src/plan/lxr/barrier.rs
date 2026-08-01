//! Read/Write barrier implementations.

use std::sync::Arc;

use atomic::Ordering;

use super::LazySweepingJobsCounter;
use super::LXR;
use crate::plan::barriers::BarrierSemantics;
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::plan::lxr::gc_work::rc::ProcessDecs;
use crate::plan::lxr::gc_work::rc::ProcessIncs;
use crate::plan::lxr::gc_work::rc::EDGE_KIND_MATURE;
use crate::plan::lxr::gc_work::tracing::ProcessModBufSATB;
use crate::plan::lxr::SkipReason;
use crate::plan::VectorQueue;
use crate::scheduler::WorkBucketStage;
use crate::util::metadata::log_bit::{LOGGED_VALUE, UNLOGGED_VALUE};
use crate::util::metadata::side_metadata::address_to_meta_address;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::*;
use crate::vm::slot::MemorySlice;
use crate::vm::slot::Slot;
use crate::vm::*;
use crate::MMTK;

pub struct LXRFieldBarrierSemantics<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
    tls: VMMutatorThread,
    incs: VectorQueue<VM::VMSlot>,
    decs: VectorQueue<ObjectReference>,
    refs: VectorQueue<ObjectReference>,
    lxr: &'static LXR<VM>,
    /// Which barrier path pushed into `decs`, for diagnostics.
    dec_origin: &'static str,
}

impl<VM: VMBinding> LXRFieldBarrierSemantics<VM> {
    const UNLOG_BITS: SideMetadataSpec = *VM::VMObjectModel::GLOBAL_FIELD_UNLOG_BIT_SPEC
        .as_spec()
        .extract_side_spec();

    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<VM>, tls: VMMutatorThread) -> Self {
        Self {
            mmtk,
            tls,
            incs: VectorQueue::default(),
            decs: VectorQueue::default(),
            refs: VectorQueue::default(),
            lxr: mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap(),
            dec_origin: "barrier-unknown",
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

    /// Whether this slot has a field unlog bit to consult.
    ///
    /// A slot only has one if it lies within the heap MMTk reserved side metadata for.
    /// A VM may hand the barrier slots outside it: Julia's arrays can keep their
    /// elements in a malloc'd buffer, so scanning an array yields slots in memory MMTk
    /// never mapped metadata for, and computing a metadata address from one of those
    /// reads unmapped memory.
    fn slot_has_unlog_bit(slot: VM::VMSlot) -> bool {
        // Ask whether the field's unlog bit actually exists, by checking that the metadata
        // address derived from the slot is mapped.
        //
        // This used to be an interval test against `vm_layout().heap_start/heap_end`, which
        // is not the same question: spaces mapped outside that range -- the VM space in
        // particular -- have perfectly good metadata but fail the interval test, so their
        // objects were misclassified as living outside the heap and pushed down the
        // out-of-heap path.
        address_to_meta_address(&Self::UNLOG_BITS, slot.to_address()).is_mapped()
    }

    fn log_slot_and_get_old_target(&self, slot: VM::VMSlot) -> Result<Option<ObjectReference>, ()> {
        if !Self::slot_has_unlog_bit(slot) {
            // No bit to coalesce on, so record the write every time rather than skipping
            // it. NOT correct: the slot is re-read when its increment is processed, and
            // nothing keeps memory outside the heap alive until then. Observed to be
            // millions of stack addresses per GC cycle.
            probe!(mmtk, lxr_slot_skipped, SkipReason::OutOfHeap);
            // Short-circuits before the atomic unless the check is actually on.
            if crate::plan::lxr::check_incs()
                && crate::plan::lxr::OUT_OF_HEAP_SLOTS.fetch_add(1, Ordering::Relaxed) == 0
            {
                eprintln!(
                    "[lxr] first out-of-heap barrier slot at {}; captured from:\n{}",
                    slot.to_address(),
                    std::backtrace::Backtrace::force_capture()
                );
            }
            return Ok(slot.load());
        }
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

    fn slow(
        &mut self,
        _src: Option<ObjectReference>,
        slot: VM::VMSlot,
        old: Option<ObjectReference>,
    ) {
        // Reference counting
        if let Some(old) = old {
            crate::plan::lxr::record_rc_event(old, "dec/barrier", _src, slot.to_address());
            self.dec_origin = "barrier-field";
            self.decs.push(old);
            if self.decs.is_full() {
                self.flush_decs_and_satb();
            }
        }
        // A tracepoint rather than a counter: this runs on every reference store that
        // reaches the barrier slow path, and a global atomic here is one cache line
        // ping-ponging between every mutator thread in the program.
        probe!(mmtk, lxr_inc_pushed, slot.to_address().as_usize());
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
            && (self.lxr.concurrent_work_in_progress()
                || self.lxr.current_pause() == Some(Pause::FinalMark))
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
            let origin = self.dec_origin;
            let w = if self.should_create_satb_packets() {
                let decs = Arc::new(self.decs.take());
                self.mmtk.scheduler.work_buckets[WorkBucketStage::FinishConcurrentWork]
                    .add(ProcessModBufSATB::new_arc(decs.clone()));
                {
                    let mut w = ProcessDecs::new_arc(decs, LazySweepingJobsCounter::new_decs());
                    w.origin = origin;
                    w
                }
            } else {
                let decs = self.decs.take();
                {
                    let mut w = ProcessDecs::new(decs, LazySweepingJobsCounter::new_decs());
                    w.origin = origin;
                    w
                }
            };
            if super::LAZY_DECREMENTS {
                self.mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent]
                    .add_deferred(Box::new(w));
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
            cursor += 16usize;
        }
        if all_logged {
            return;
        }

        for s in dst.iter_slots() {
            let _succ = self.enqueue_node(None, s, None);
        }
    }

    fn load_weak_reference(&mut self, o: ObjectReference) {
        if !self.lxr.concurrent_work_in_progress() || self.lxr.is_marked(o) {
            return;
        }
        self.refs.push(o);
        if self.refs.is_full() {
            self.flush_weak_refs();
        }
    }

    fn object_probable_write_slow(&mut self, obj: ObjectReference) {
        // Recording a slot means re-reading it at the pause, which needs the slot to still
        // be there and still be the same field. That does not hold for fields outside the
        // heap: Julia keeps a large `jl_genericmemory`'s elements in a malloc'd buffer, and
        // is free to realloc or free it before the pause, at which point the increment
        // reads whatever now occupies that address.
        //
        // For such an object, buffer the object rather than its slots and re-derive the
        // fields at the pause. That keeps the coalescing semantics exactly -- the values
        // read then are still the epoch-end values -- while making a realloc harmless,
        // since the object is asked for its fields again. The decrements of the old values
        // are pushed here either way, which is where they have to happen.
        obj.iterate_fields::<VM, _>(self.tls.0, |s| {
            if !Self::slot_has_unlog_bit(s) {
                probe!(mmtk, lxr_slot_skipped, SkipReason::OutOfHeap);
                return;
            }
            // A derived slot cannot survive the trip to the pause: it is only interpretable
            // together with an offset captured now, and the field it describes is free to
            // change before then. It also names an object some other slot already names, so
            // skipping it costs no reachability. See `Slot::is_derived`.
            if s.is_derived() {
                probe!(mmtk, lxr_slot_skipped, SkipReason::Derived);
                return;
            }
            let _succ = self.enqueue_node(Some(obj), s, None);
        });
    }
}
