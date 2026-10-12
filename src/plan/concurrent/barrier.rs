use std::sync::atomic::Ordering;

use super::{concurrent_marking_work::ProcessModBufSATB, Pause};
use crate::plan::global::PlanTraceObject;
use crate::policy::gc_work::TraceKind;
use crate::util::VMMutatorThread;
use crate::{
    plan::{barriers::BarrierSemantics, concurrent::global::ConcurrentPlan, VectorQueue},
    scheduler::{GCWork, WorkBucketStage},
    util::ObjectReference,
    vm::{
        slot::{MemorySlice, Slot},
        VMBinding,
    },
    MMTK,
};

pub struct SATBBarrierSemantics<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    mmtk: &'static MMTK<VM>,
    tls: VMMutatorThread,
    satb: VectorQueue<ObjectReference>,
    refs: VectorQueue<ObjectReference>,
    plan: &'static P,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    SATBBarrierSemantics<VM, P, KIND>
{
    pub fn new(mmtk: &'static MMTK<VM>, tls: VMMutatorThread) -> Self {
        Self {
            mmtk,
            tls,
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
                self.add_work(ProcessModBufSATB::<VM, P, KIND>::new(satb));
            } else {
                let _ = self.satb.take();
            };
        }
    }

    #[cold]
    fn flush_weak_refs(&mut self) {
        if !self.refs.is_empty() {
            let nodes = self.refs.take();
            self.add_work(ProcessModBufSATB::<VM, P, KIND>::new(nodes));
        }
    }

    fn add_work(&self, work: impl GCWork<VM>) {
        let bucket_stage = if self.plan.concurrent_work_in_progress() {
            WorkBucketStage::Concurrent
        } else {
            debug_assert_ne!(self.plan.current_pause(), Some(Pause::InitialMark));
            WorkBucketStage::Closure
        };
        let bucket = &self.mmtk.scheduler.work_buckets[bucket_stage];
        // If the bucket is disabled, we still add the work, but we dont need to notify a worker.
        if bucket.is_enabled() {
            // If there is a race, and the bucket is disabled now, it is fine.
            // We just additionally notify the worker, which is harmless.
            bucket.add(work);
        } else {
            bucket.add_no_notify(work);
        }
    }

    fn should_create_satb_packets(&self) -> bool {
        self.plan.concurrent_work_in_progress()
            || self.plan.current_pause() == Some(Pause::FinalMark)
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    BarrierSemantics for SATBBarrierSemantics<VM, P, KIND>
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
        obj.iterate_fields::<VM, _>(self.tls.0, |s| {
            self.enqueue_node(Some(obj), s, None);
        });
    }
}

/// The slow-path semantics of `HybridBarrier`.
///
/// It buffers the objects to shade, like Go's per-P `wbBuf` (`runtime/mwbbuf.go`), and flushes
/// the buffer into a [`ProcessModBufSATB`] packet, which marks every unmarked object in it and
/// traces from it.  That is Go's `wbBufFlush1`.
pub struct HybridBarrierSemantics<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    mmtk: &'static MMTK<VM>,
    tls: VMMutatorThread,
    /// Objects to shade: old and new values of reference stores, and loaded weak referents.
    buf: Vec<ObjectReference>,
    /// Objects whose fields may be initialised without a barrier
    /// ([`crate::plan::barriers::Barrier::object_probable_write`]).  Their fields are traced when
    /// the buffers are flushed from outside the mutator, after those stores have happened.
    rescan: Vec<ObjectReference>,
    plan: &'static P,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    HybridBarrierSemantics<VM, P, KIND>
{
    /// The capacity of the buffer.  Go's `wbBuf` holds 512 pointers (`runtime/mwbbuf.go`).
    pub const BUFFER_SIZE: usize = 512;

    pub fn new(mmtk: &'static MMTK<VM>, tls: VMMutatorThread) -> Self {
        Self {
            mmtk,
            tls,
            buf: Vec::with_capacity(Self::BUFFER_SIZE),
            rescan: vec![],
            plan: mmtk.get_plan().downcast_ref::<P>().unwrap(),
        }
    }

    #[inline(always)]
    fn shade(&mut self, object: ObjectReference) {
        self.buf.push(object);
        if self.buf.len() >= Self::BUFFER_SIZE {
            self.flush_buf();
        }
    }

    /// The objects currently buffered for shading.
    #[cfg(test)]
    pub(crate) fn buffered(&self) -> &[ObjectReference] {
        &self.buf
    }

    /// Return `true` if the buffers hold any work.  Used to detect mark termination.
    #[allow(dead_code)] // Used once ConcurrentSpan detects mark termination.
    pub fn has_work(&self) -> bool {
        !self.buf.is_empty() || !self.rescan.is_empty()
    }

    #[cold]
    fn flush_buf(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        let objects = std::mem::replace(&mut self.buf, Vec::with_capacity(Self::BUFFER_SIZE));
        if self.should_create_packets() {
            self.add_work(ProcessModBufSATB::<VM, P, KIND>::new(objects));
        }
    }

    fn flush_rescan(&mut self) {
        if self.rescan.is_empty() {
            return;
        }
        let objects = std::mem::take(&mut self.rescan);
        if self.should_create_packets() {
            // The objects are allocated black, so they are already marked.  Scan their fields.
            self.add_work(super::concurrent_marking_work::ConcurrentTraceObjects::<
                VM,
                P,
                KIND,
            >::new(objects, true));
        }
    }

    fn add_work(&self, work: impl GCWork<VM>) {
        let bucket_stage = if self.plan.concurrent_work_in_progress() {
            WorkBucketStage::Concurrent
        } else {
            debug_assert_ne!(self.plan.current_pause(), Some(Pause::InitialMark));
            WorkBucketStage::Closure
        };
        let bucket = &self.mmtk.scheduler.work_buckets[bucket_stage];
        if bucket.is_enabled() {
            bucket.add(work);
        } else {
            bucket.add_no_notify(work);
        }
    }

    fn should_create_packets(&self) -> bool {
        self.plan.concurrent_work_in_progress()
            || self.plan.current_pause() == Some(Pause::FinalMark)
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    BarrierSemantics for HybridBarrierSemantics<VM, P, KIND>
{
    type VM = VM;

    #[cold]
    fn flush(&mut self) {
        self.flush_buf();
        self.flush_rescan();
    }

    /// Go: `runtime/mwbbuf.go:221-227` (2c3c1caad2) records both the new and the old value.
    fn object_reference_write_slow(
        &mut self,
        _src: ObjectReference,
        slot: <Self::VM as VMBinding>::VMSlot,
        target: Option<ObjectReference>,
    ) {
        if let Some(old) = slot.load() {
            self.shade(old);
        }
        if let Some(new) = target {
            self.shade(new);
        }
    }

    /// Go's `bulkBarrierPreWrite` (`runtime/mbitmap.go:388-466`): shade the old values in the
    /// destination and the values in the source.
    fn memory_region_copy_slow(
        &mut self,
        src: <Self::VM as VMBinding>::VMMemorySlice,
        dst: <Self::VM as VMBinding>::VMMemorySlice,
    ) {
        for s in dst.iter_slots() {
            if let Some(old) = s.load() {
                self.shade(old);
            }
        }
        for s in src.iter_slots() {
            if let Some(new) = s.load() {
                self.shade(new);
            }
        }
    }

    /// The object's fields are about to be written without a barrier.  Shade the old values now
    /// (deletion half), and trace the fields again later to catch the new values (insertion half).
    fn object_probable_write_slow(&mut self, obj: ObjectReference) {
        obj.iterate_fields::<VM, _>(self.tls.0, |s| {
            if let Some(old) = s.load() {
                self.shade(old);
            }
        });
        self.rescan.push(obj);
    }

    /// Shade the referent of a weak reference that is loaded while marking.  That is Go's
    /// `shade(ptr)` in `makeStrongFromWeak` (`runtime/mheap.go:2459-2466`).
    fn load_weak_reference(&mut self, o: ObjectReference) {
        self.shade(o);
    }
}
