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

pub struct GenObjectBarrier<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
    gen: &'static Gen<VM>,
    modbuf: Vec<ObjectReference>,
    capacity: usize,
    arraycopy_modbuf: Vec<(Address, usize)>,
}

impl<VM: VMBinding> GenObjectBarrier<VM> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<VM>, gen: &'static Gen<VM>, capacity: usize) -> Self {
        Self {
            mmtk,
            gen,
            modbuf: vec![],
            capacity,
            arraycopy_modbuf: vec![],
        }
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

    #[inline(always)]
    fn try_record_node(&mut self, obj: ObjectReference) {
        // If the objecct is unlogged, log it and push it to mod buffer
        if self.log_object(obj) {
            // enqueue
            self.modbuf.push(obj);
            if self.modbuf.len() >= self.capacity {
                self.flush();
            }
        }
    }

    #[inline(always)]
    pub fn gen_object_reference_write_slow(&mut self, src: ObjectReference) {
        self.try_record_node(src)
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
        self.try_record_node(src);
    }

    #[inline(always)]
    fn array_copy_pre(&mut self, _src: Address, dst: Address, count: usize) {
        debug_assert!(!dst.is_zero());
        if !self.gen.nursery.address_in_space(dst) {
            self.arraycopy_modbuf.push((dst, count));
            if self.arraycopy_modbuf.len() >= self.capacity {
                self.flush();
            }
        }
    }
}
