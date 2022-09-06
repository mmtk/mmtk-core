//! Read/Write barrier implementations.

use atomic::Ordering;

use crate::scheduler::gc_work::*;
use crate::scheduler::WorkBucketStage;
use crate::util::metadata::MetadataSpec;
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

/// For field writes in HotSpot, we cannot always get the source object pointer and the field address
pub enum BarrierWriteTarget {
    Object(ObjectReference),
    Slot(Address),
}

pub trait Barrier: 'static + Send {
    fn flush(&mut self);
    fn post_write_barrier(&mut self, target: BarrierWriteTarget);
    fn post_write_barrier_slow(&mut self, target: BarrierWriteTarget);
}

pub struct NoBarrier;

impl Barrier for NoBarrier {
    fn flush(&mut self) {}
    fn post_write_barrier(&mut self, _target: BarrierWriteTarget) {}
    fn post_write_barrier_slow(&mut self, _target: BarrierWriteTarget) {}
}

pub struct ObjectRememberingBarrier<E: ProcessEdgesWork> {
    mmtk: &'static MMTK<E::VM>,
    modbuf: Vec<ObjectReference>,
    /// The metadata used for log bit. Though this allows taking an arbitrary metadata spec,
    /// for this field, 0 means logged, and 1 means unlogged (the same as the vm::object_model::VMGlobalLogBitSpec).
    meta: MetadataSpec,
}

impl<E: ProcessEdgesWork> ObjectRememberingBarrier<E> {
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
            // Try set the bit from 1 to 0 (log object). This may fail, if
            // 1. the bit is cleared by others, or
            // 2. other bits in the same byte may get modified if we use side metadata
            if self.meta.compare_exchange_metadata::<E::VM, u8>(
                object,
                1,
                0,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                // We just logged the object
                return true;
            } else {
                let old_value = self
                    .meta
                    .load_atomic::<E::VM, u8>(object, None, Ordering::SeqCst);
                // If the bit is cleared before, someone else has logged the object. Return false.
                if old_value == 0 {
                    return false;
                }
            }
        }
    }

    #[inline(always)]
    fn enqueue_node(&mut self, obj: ObjectReference) {
        // If the objecct is unlogged, log it and push it to mod buffer
        if self.log_object(obj) {
            self.modbuf.push(obj);
            if self.modbuf.len() >= E::CAPACITY {
                self.flush();
            }
        }
    }

    #[inline(always)]
    fn barrier(&mut self, obj: ObjectReference) {
        if unsafe { self.meta.load::<E::VM, u8>(obj, None) == 0 } {
            return;
        }
        self.barrier_slow(obj);
    }

    #[inline(never)]
    fn barrier_slow(&mut self, obj: ObjectReference) {
        self.enqueue_node(obj);
    }
}

impl<E: ProcessEdgesWork> Barrier for ObjectRememberingBarrier<E> {
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
    fn post_write_barrier(&mut self, target: BarrierWriteTarget) {
        match target {
            BarrierWriteTarget::Object(obj) => self.barrier(obj),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn post_write_barrier_slow(&mut self, target: BarrierWriteTarget) {
        match target {
            BarrierWriteTarget::Object(obj) => {
                self.enqueue_node(obj);
            }
            _ => unreachable!(),
        }
    }
}
