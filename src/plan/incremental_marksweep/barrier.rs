//! IncrementalMarkSweep SATB barrier.

use super::global::IncrementalMarkSweep;
use crate::plan::barriers::BarrierSemantics;
use crate::plan::VectorQueue;
use crate::util::{ObjectReference, VMMutatorThread};
use crate::vm::slot::{MemorySlice, Slot};
use crate::vm::VMBinding;
use crate::MMTK;
use std::sync::atomic::Ordering;

pub struct IncrementalMarkSweepBarrierSemantics<VM: VMBinding> {
    tls: VMMutatorThread,
    plan: &'static IncrementalMarkSweep<VM>,
    satb: VectorQueue<ObjectReference>,
}

impl<VM: VMBinding> IncrementalMarkSweepBarrierSemantics<VM> {
    pub fn new(mmtk: &'static MMTK<VM>, tls: VMMutatorThread) -> Self {
        Self {
            tls,
            plan: mmtk
                .get_plan()
                .downcast_ref::<IncrementalMarkSweep<VM>>()
                .unwrap(),
            satb: VectorQueue::new(),
        }
    }

    fn push(&mut self, o: ObjectReference) {
        self.satb.push(o);
        if self.satb.is_full() {
            self.flush_satb();
        }
    }

    fn flush_satb(&mut self) {
        let buf = self.satb.take();
        if !buf.is_empty() && self.plan.is_marking() {
            self.plan.satb.push(buf);
        }
    }

    fn log_fields(&mut self, src: ObjectReference) {
        let tls = self.tls.0;
        src.iterate_fields::<VM, _>(tls, |s: VM::VMSlot| {
            if let Some(old) = s.load() {
                self.push(old);
            }
        });
        Self::UNLOG_BIT_SPEC.store_atomic::<VM, u8>(src, 0, None, Ordering::SeqCst);
    }
}

impl<VM: VMBinding> BarrierSemantics for IncrementalMarkSweepBarrierSemantics<VM> {
    type VM = VM;

    fn flush(&mut self) {
        self.flush_satb();
    }

    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        _slot: VM::VMSlot,
        _target: Option<ObjectReference>,
    ) {
        self.log_fields(src);
    }

    fn memory_region_copy_slow(&mut self, _src: VM::VMMemorySlice, dst: VM::VMMemorySlice) {
        if !self.plan.is_marking() {
            return;
        }
        for s in dst.iter_slots() {
            if let Some(old) = s.load() {
                self.push(old);
            }
        }
    }

    fn load_weak_reference(&mut self, referent: ObjectReference) {
        if self.plan.is_marking() {
            self.push(referent);
        }
    }

    fn object_probable_write_slow(&mut self, obj: ObjectReference) {
        if Self::UNLOG_BIT_SPEC.load_atomic::<VM, u8>(obj, None, Ordering::SeqCst) != 0 {
            self.log_fields(obj);
        }
    }
}
