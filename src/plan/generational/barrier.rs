//! Generational read/write barrier implementations.

use crate::plan::barriers::BarrierSemantics;
use crate::plan::{Queue, VectorQueue};
use crate::policy::space::Space;
use crate::scheduler::WorkBucketStage;
use crate::util::constants::BYTES_IN_ADDRESS;
use crate::util::constants::LOG_BYTES_IN_ADDRESS;
use crate::util::*;
use crate::vm::VMBinding;
use crate::MMTK;

use super::gc_work::GenNurseryProcessEdges;
use super::gc_work::ProcessModBuf;
use super::gc_work::ProcessRegionModBuf;
use super::global::Gen;

pub struct GenObjectBarrierSemantics<VM: VMBinding> {
    /// MMTk instance
    mmtk: &'static MMTK<VM>,
    /// Generational plan
    gen: &'static Gen<VM>,
    /// Object modbuf. Contains a list of objects that may contain pointers to the nursery space.
    modbuf: VectorQueue<ObjectReference>,
    /// Array-copy modbuf. Contains a list of sub-arrays or array slices that may contain pointers to the nursery space.
    region_modbuf: VectorQueue<(Address, usize)>,
}

impl<VM: VMBinding> GenObjectBarrierSemantics<VM> {
    pub fn new(mmtk: &'static MMTK<VM>, gen: &'static Gen<VM>) -> Self {
        Self {
            mmtk,
            gen,
            modbuf: VectorQueue::new(),
            region_modbuf: VectorQueue::new(),
        }
    }

    #[cold]
    fn flush_modbuf(&mut self) {
        if let Some(buf) = self.modbuf.take() {
            debug_assert!(!buf.is_empty());
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure]
                .add(ProcessModBuf::<GenNurseryProcessEdges<VM>>::new(buf));
        }
    }

    #[cold]
    fn flush_region_modbuf(&mut self) {
        if let Some(buf) = self.region_modbuf.take() {
            debug_assert!(!buf.is_empty());
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure]
                .add(ProcessRegionModBuf::<GenNurseryProcessEdges<VM>>::new(buf));
        }
    }
}

impl<VM: VMBinding> BarrierSemantics for GenObjectBarrierSemantics<VM> {
    type VM = VM;

    #[cold]
    fn flush(&mut self) {
        self.flush_modbuf();
        self.flush_region_modbuf();
    }

    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
        // enqueue the object
        self.modbuf.enqueue(src);
        self.modbuf.is_full().then(|| self.flush_modbuf());
    }

    fn memory_region_copy_slow(&mut self, _src: Address, dst: Address, bytes: usize) {
        debug_assert!(!dst.is_zero());
        // Only enqueue array slices in mature spaces
        if !self.gen.nursery.address_in_space(dst) {
            // enqueue
            debug_assert_eq!(
                bytes & (BYTES_IN_ADDRESS - 1),
                0,
                "bytes should be a multiple of words"
            );
            self.region_modbuf
                .enqueue((dst, bytes >> LOG_BYTES_IN_ADDRESS));
            self.region_modbuf
                .is_full()
                .then(|| self.flush_region_modbuf());
        }
    }
}
