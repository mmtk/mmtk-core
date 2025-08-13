use std::sync::atomic::Ordering;

use super::{concurrent_marking_work::ProcessModBufSATB, Pause};
use crate::plan::global::PlanTraceObject;
use crate::{
    plan::{barriers::BarrierSemantics, concurrent::global::ConcurrentPlan, VectorQueue},
    scheduler::WorkBucketStage,
    util::ObjectReference,
    vm::{
        slot::{MemorySlice, Slot},
        VMBinding,
    },
    MMTK,
};

pub struct SATBBarrierSemantics<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>> {
    mmtk: &'static MMTK<VM>,
    satb: VectorQueue<ObjectReference>,
    refs: VectorQueue<ObjectReference>,
    plan: &'static P,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>> SATBBarrierSemantics<VM, P> {
    pub fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            mmtk,
            satb: VectorQueue::default(),
            refs: VectorQueue::default(),
            plan: mmtk.get_plan().downcast_ref::<P>().unwrap(),
        }
    }

    fn slow(&mut self, _src: Option<ObjectReference>, _slot: VM::VMSlot, old: ObjectReference) {
        self.satb.push(old);
        if self.satb.is_full() {
            self.flush_satb();
        }
    }

    fn enqueue_node(
        &mut self,
        src: Option<ObjectReference>,
        slot: VM::VMSlot,
        _new: Option<ObjectReference>,
    ) -> bool {
        if let Some(old) = slot.load() {
            self.slow(src, slot, old);
        }
        true
    }

    /// Attempt to atomically log an object.
    /// Returns true if the object is not logged previously.
    fn log_object(&self, object: ObjectReference) -> bool {
        Self::UNLOG_BIT_SPEC.store_atomic::<VM, u8>(object, 0, None, Ordering::SeqCst);
        true
    }

    fn flush_satb(&mut self) {
        if !self.satb.is_empty() {
            if self.should_create_satb_packets() {
                let satb = self.satb.take();
                let bucket = if self.plan.concurrent_work_in_progress() {
                    WorkBucketStage::Unconstrained
                } else {
                    WorkBucketStage::Closure
                };
                self.mmtk.scheduler.work_buckets[bucket].add(ProcessModBufSATB::<VM, P>::new(satb));
            } else {
                let _ = self.satb.take();
            };
        }
    }

    #[cold]
    fn flush_weak_refs(&mut self) {
        if !self.refs.is_empty() {
            // debug_assert!(self.should_create_satb_packets());
            let nodes = self.refs.take();
            let bucket = if self.plan.concurrent_work_in_progress() {
                WorkBucketStage::Unconstrained
            } else {
                WorkBucketStage::Closure
            };
            self.mmtk.scheduler.work_buckets[bucket].add(ProcessModBufSATB::<VM, P>::new(nodes));
        }
    }

    fn should_create_satb_packets(&self) -> bool {
        self.plan.concurrent_work_in_progress()
            || self.plan.current_pause() == Some(Pause::FinalMark)
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>> BarrierSemantics
    for SATBBarrierSemantics<VM, P>
{
    type VM = VM;

    #[cold]
    fn flush(&mut self) {
        self.flush_satb();
        self.flush_weak_refs();
    }

    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        _slot: <Self::VM as VMBinding>::VMSlot,
        _target: Option<ObjectReference>,
    ) {
        self.object_probable_write_slow(src);
        self.log_object(src);
    }

    fn memory_region_copy_slow(
        &mut self,
        _src: <Self::VM as VMBinding>::VMMemorySlice,
        dst: <Self::VM as VMBinding>::VMMemorySlice,
    ) {
        for s in dst.iter_slots() {
            self.enqueue_node(None, s, None);
        }
    }

    /// Enqueue the referent during concurrent marking.
    ///
    /// Note: During concurrent marking, a collector based on snapshot-at-the-beginning (SATB) will
    /// not reach objects that were weakly reachable at the time of `InitialMark`.  But if a mutator
    /// loads from a weak reference field during concurrent marking, it will make the referent
    /// strongly reachable, yet the referent is still not part of the SATB.  We must conservatively
    /// enqueue the referent even though its reachability has not yet been established, otherwise it
    /// (and its children) may be treated as garbage if it happened to be weakly reachable at the
    /// time of `InitialMark`.
    fn load_weak_reference(&mut self, o: ObjectReference) {
        if !self.plan.concurrent_work_in_progress() {
            return;
        }
        self.refs.push(o);
        if self.refs.is_full() {
            self.flush_weak_refs();
        }
    }

    fn object_probable_write_slow(&mut self, obj: ObjectReference) {
        obj.iterate_fields::<VM, _>(|s| {
            self.enqueue_node(Some(obj), s, None);
        });
    }
}
