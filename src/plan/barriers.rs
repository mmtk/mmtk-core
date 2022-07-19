//! Read/Write barrier implementations.

use atomic::Ordering;

use crate::scheduler::gc_work::*;
use crate::scheduler::WorkBucketStage;
use crate::util::metadata::load_metadata;
use crate::util::metadata::{compare_exchange_metadata, MetadataSpec};
use crate::util::*;
use crate::MMTK;

/// BarrierSelector describes which barrier to use.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BarrierSelector {
    NoBarrier,
    ObjectBarrier,
}

impl BarrierSelector {
    pub const fn equals(&self, other: BarrierSelector) -> bool {
        // cast enum to u8 then compare. Otherwise, we cannot do it in a const fn.
        *self as u8 == other as u8
    }
}

pub trait Barrier: 'static + Send {
    fn flush(&mut self) {}

    fn object_reference_write(
        &mut self,
        src: ObjectReference,
        slot: Address,
        target: ObjectReference,
    ) {
        self.object_reference_write_pre(src, slot, target);
        unsafe { slot.store(target) };
        self.object_reference_write_post(src, slot, target);
    }

    fn object_reference_write_pre(
        &mut self,
        _src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
    }

    fn object_reference_write_post(
        &mut self,
        _src: ObjectReference,
        _slot: Address,
        _target: ObjectReference,
    ) {
    }

    fn array_copy(
        &mut self,
        src_object: Option<ObjectReference>,
        src: Address,
        dst_object: Option<ObjectReference>,
        dst: Address,
        count: usize,
    ) {
        self.array_copy_pre(src_object, src, dst_object, dst, count);
        unsafe { std::ptr::copy::<ObjectReference>(src.to_ptr(), dst.to_mut_ptr(), count) };
        self.array_copy_post(src_object, src, dst_object, dst, count);
    }

    fn array_copy_pre(
        &mut self,
        _src_object: Option<ObjectReference>,
        _src: Address,
        _dst_object: Option<ObjectReference>,
        _dst: Address,
        _count: usize,
    ) {
    }

    fn array_copy_post(
        &mut self,
        _src_object: Option<ObjectReference>,
        _src: Address,
        _dst_object: Option<ObjectReference>,
        _dst: Address,
        _count: usize,
    ) {
    }
}

pub struct NoBarrier;

impl Barrier for NoBarrier {}

pub struct ObjectBarrier<E: ProcessEdgesWork> {
    mmtk: &'static MMTK<E::VM>,
    modbuf: Vec<ObjectReference>,
    /// The metadata used for log bit. Though this allows taking an arbitrary metadata spec,
    /// for this field, 0 means logged, and 1 means unlogged (the same as the vm::object_model::VMGlobalLogBitSpec).
    meta: MetadataSpec,
}

impl<E: ProcessEdgesWork> ObjectBarrier<E> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<E::VM>, meta: MetadataSpec) -> Self {
        Self {
            mmtk,
            modbuf: vec![],
            meta,
        }
    }

    /// Attepmt to atomically log an object.
    /// Returns true if the object is not logged previously.
    #[inline(always)]
    fn log_object(&self, object: ObjectReference) -> bool {
        loop {
            let old_value =
                load_metadata::<E::VM>(&self.meta, object, None, Some(Ordering::SeqCst));
            if old_value == 0 {
                return false;
            }
            if compare_exchange_metadata::<E::VM>(
                &self.meta,
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
            if self.modbuf.len() >= E::CAPACITY {
                self.flush();
            }
        }
    }
}

impl<E: ProcessEdgesWork> Barrier for ObjectBarrier<E> {
    #[cold]
    fn flush(&mut self) {
        let mut modbuf = vec![];
        std::mem::swap(&mut modbuf, &mut self.modbuf);
        debug_assert!(
            !self.mmtk.scheduler.work_buckets[WorkBucketStage::Final].is_activated(),
            "{:?}",
            self as *const _
        );
        if !modbuf.is_empty() {
            self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure]
                .add(ProcessModBuf::<E>::new(modbuf, self.meta));
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
    fn array_copy_pre(
        &mut self,
        _src_object: Option<ObjectReference>,
        _src: Address,
        dst_object: Option<ObjectReference>,
        _dst: Address,
        _count: usize,
    ) {
        self.try_record_node(dst_object.unwrap());
    }
}
