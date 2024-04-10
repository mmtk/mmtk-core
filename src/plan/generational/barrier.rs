//! Generational read/write barrier implementations.

use crate::plan::barriers::BarrierSemantics;
use crate::plan::PlanTraceObject;
use crate::plan::VectorQueue;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::scheduler::WorkBucketStage;
use crate::util::constants::BYTES_IN_INT;
use crate::util::*;
use crate::vm::edge_shape::MemorySlice;
use crate::vm::VMBinding;
use crate::MMTK;

use super::gc_work::GenNurseryProcessEdges;
use super::gc_work::ProcessModBuf;
use super::gc_work::ProcessRegionModBuf;
use super::global::GenerationalPlanExt;

pub struct GenObjectBarrierSemantics<
    VM: VMBinding,
    P: GenerationalPlanExt<VM> + PlanTraceObject<VM>,
> {
    /// MMTk instance
    mmtk: &'static MMTK<VM>,
    /// Generational plan
    plan: &'static P,
    /// Object modbuf. Contains a list of objects that may contain pointers to the nursery space.
    modbuf: VectorQueue<ObjectReference>,
    /// Array-copy modbuf. Contains a list of sub-arrays or array slices that may contain pointers to the nursery space.
    region_modbuf: VectorQueue<VM::VMMemorySlice>,
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>>
    GenObjectBarrierSemantics<VM, P>
{
    pub fn new(mmtk: &'static MMTK<VM>, plan: &'static P) -> Self {
        Self {
            mmtk,
            plan,
            modbuf: VectorQueue::new(),
            region_modbuf: VectorQueue::new(),
        }
    }

    fn flush_modbuf(&mut self) {
        let buf = self.modbuf.take();
        if !buf.is_empty() {
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure]
                .add(ProcessModBuf::<GenNurseryProcessEdges<VM, P, DEFAULT_TRACE>>::new(buf));
        }
    }

    fn flush_region_modbuf(&mut self) {
        let buf = self.region_modbuf.take();
        if !buf.is_empty() {
            debug_assert!(!buf.is_empty());
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure].add(ProcessRegionModBuf::<
                GenNurseryProcessEdges<VM, P, DEFAULT_TRACE>,
            >::new(buf));
        }
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>> BarrierSemantics
    for GenObjectBarrierSemantics<VM, P>
{
    type VM = VM;

    fn flush(&mut self) {
        self.flush_modbuf();
        self.flush_region_modbuf();
    }

    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        _slot: VM::VMEdge,
        _target: ObjectReference,
    ) {
        // enqueue the object
        self.modbuf.push(src);
        self.modbuf.is_full().then(|| self.flush_modbuf());
    }

    fn memory_region_copy_slow(&mut self, _src: VM::VMMemorySlice, dst: VM::VMMemorySlice) {
        // Check if the destination object/slice is in nursery space.
        let dst_in_nursery = match dst.object() {
            Some(obj) => self.plan.is_object_in_nursery(obj),
            None => self.plan.is_address_in_nursery(dst.start()),
        };
        // Only enqueue array slices in mature spaces
        if !dst_in_nursery {
            // enqueue
            debug_assert_eq!(
                dst.bytes() & (BYTES_IN_INT - 1),
                0,
                "bytes should be a multiple of 32-bit words"
            );
            self.region_modbuf.push(dst);
            self.region_modbuf
                .is_full()
                .then(|| self.flush_region_modbuf());
        }
    }

    fn object_probable_write_slow(&mut self, obj: ObjectReference) {
        // enqueue the object
        self.modbuf.push(obj);
        self.modbuf.is_full().then(|| self.flush_modbuf());
    }
}
