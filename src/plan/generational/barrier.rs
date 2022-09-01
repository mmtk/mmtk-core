//! Generational read/write barrier implementations.

use crate::plan::barriers::BarrierSemantics;
use crate::policy::space::Space;
use crate::scheduler::ProcessEdgesWork;
use crate::scheduler::WorkBucketStage;
use crate::util::*;
use crate::vm::VMBinding;
use crate::MMTK;

use super::gc_work::GenNurseryProcessEdges;
use super::gc_work::ProcessArrayCopyModBuf;
use super::gc_work::ProcessModBuf;
use super::global::Gen;

pub struct GenObjectBarrierSemantics<VM: VMBinding> {
    /// MMTk instance
    mmtk: &'static MMTK<VM>,
    /// Generational plan
    gen: &'static Gen<VM>,
    /// Object modbuf. Contains a list of objects that may contain pointers to the nursery space.
    modbuf: Vec<ObjectReference>,
    /// Array-copy modbuf. Contains a list of sub-arrays or array slices that may contain pointers to the nursery space.
    arraycopy_modbuf: Vec<(Address, usize)>,
    /// Max size of the modbuf(s).
    capacity: usize,
}

impl<VM: VMBinding> GenObjectBarrierSemantics<VM> {
    pub fn new(mmtk: &'static MMTK<VM>, gen: &'static Gen<VM>) -> Self {
        Self {
            mmtk,
            gen,
            modbuf: vec![],
            arraycopy_modbuf: vec![],
            capacity: GenNurseryProcessEdges::<VM>::CAPACITY,
        }
    }
}

impl<VM: VMBinding> BarrierSemantics for GenObjectBarrierSemantics<VM> {
    type VM = VM;

    #[cold]
    fn flush(&mut self) {
        let mut modbuf = vec![];
        std::mem::swap(&mut modbuf, &mut self.modbuf);
        if !modbuf.is_empty() {
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure].add(ProcessModBuf::<
                GenNurseryProcessEdges<VM>,
            >::new(
                modbuf
            ));
        }
        let mut modbuf = vec![];
        std::mem::swap(&mut modbuf, &mut self.arraycopy_modbuf);
        if !modbuf.is_empty() {
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure].add(
                ProcessArrayCopyModBuf::<GenNurseryProcessEdges<VM>>::new(modbuf),
            );
        }
    }

    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
        // enqueue the object
        self.modbuf.push(src);
        // the buffer is full?
        if self.modbuf.len() >= self.capacity {
            self.flush();
        }
    }

    fn array_copy_slow(&mut self, _src: Address, dst: Address, count: usize) {
        debug_assert!(!dst.is_zero());
        // Only enqueue array slices in mature spaces
        if !self.gen.nursery.address_in_space(dst) {
            // enqueue
            self.arraycopy_modbuf.push((dst, count));
            // flush if the buffer is full
            if self.arraycopy_modbuf.len() >= self.capacity {
                self.flush();
            }
        }
    }
}
