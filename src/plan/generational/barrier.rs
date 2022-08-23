//! Generational read/write barrier implementations.

use atomic::Ordering;

use crate::plan::barriers::Barrier;
use crate::policy::space::Space;
use crate::scheduler::WorkBucketStage;
use crate::util::metadata::compare_exchange_metadata;
use crate::util::metadata::load_metadata;
use crate::util::*;
use crate::vm::{ObjectModel, VMBinding};
use crate::MMTK;

use super::gc_work::GenNurseryProcessEdges;
use super::gc_work::ProcessArrayCopyModBuf;
use super::gc_work::ProcessModBuf;
use super::global::Gen;

/// Object barrier for generational collection
pub struct GenObjectBarrier<VM: VMBinding> {
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

impl<VM: VMBinding> GenObjectBarrier<VM> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<VM>, gen: &'static Gen<VM>, capacity: usize) -> Self {
        Self {
            mmtk,
            gen,
            modbuf: vec![],
            arraycopy_modbuf: vec![],
            capacity,
        }
    }

    /// Attepmt to atomically log an object.
    /// Returns true if the object is not logged previously.
    #[inline(always)]
    fn object_is_unlogged(&self, object: ObjectReference) -> bool {
        load_metadata::<VM>(&VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC, object, None, None) != 0
    }

    /// Attepmt to atomically log an object.
    /// Returns true if the object is not logged previously.
    #[inline(always)]
    fn log_object(&self, object: ObjectReference) -> bool {
        loop {
            let old_value = load_metadata::<VM>(
                &VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
                object,
                None,
                Some(Ordering::SeqCst),
            );
            if old_value == 0 {
                return false;
            }
            if compare_exchange_metadata::<VM>(
                &VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
                object,
                1,
                0,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                return true;
            }
        }
    }

    /// object barrier slow-path call
    pub fn gen_object_reference_write_slow(&mut self, src: ObjectReference) {
        // Log and enqueue the object if it is unlogged
        if self.log_object(src) {
            // enqueue the object
            self.modbuf.push(src);
            // the buffer is full?
            if self.modbuf.len() >= self.capacity {
                self.flush();
            }
        }
    }
}

impl<VM: VMBinding> Barrier for GenObjectBarrier<VM> {
    #[cold]
    fn flush(&mut self) {
        let mut modbuf = vec![];
        std::mem::swap(&mut modbuf, &mut self.modbuf);
        if !modbuf.is_empty() {
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure].add(ProcessModBuf::<
                GenNurseryProcessEdges<VM>,
            >::new(
                modbuf,
                *VM::VMObjectModel::GLOBAL_LOG_BIT_SPEC,
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

    #[inline(always)]
    fn object_reference_write_pre(
        &mut self,
        src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
        if self.object_is_unlogged(src) {
            self.gen_object_reference_write_slow(src);
        }
    }

    #[inline(always)]
    fn object_reference_write_slow(
        &mut self,
        src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
        self.gen_object_reference_write_slow(src);
    }

    #[inline(always)]
    fn array_copy_pre(&mut self, _src: Address, dst: Address, count: usize) {
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
