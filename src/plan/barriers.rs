//! Read/Write barrier implementations.

use atomic::Ordering;

use crate::scheduler::gc_work::*;
use crate::scheduler::WorkBucketStage;
use crate::util::metadata::load_metadata;
use crate::util::metadata::{compare_exchange_metadata, MetadataSpec};
use crate::util::*;
use crate::vm::VMBinding;
use crate::MMTK;

use super::GcStatus;

/// BarrierSelector describes which barrier to use.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BarrierSelector {
    NoBarrier,
    ObjectBarrier,
    FieldLoggingBarrier,
}

/// For field writes in HotSpot, we cannot always get the source object pointer and the field address\
#[derive(Debug)]
pub enum WriteTarget {
    Field { src: ObjectReference, slot: Address, val: ObjectReference },
    ArrayCopy { src: Address, dst: Address, len: usize },
    Clone { src: ObjectReference, dst: ObjectReference, size: usize },
}

pub trait Barrier: 'static + Send {
    fn flush(&mut self);
    fn assert_is_flushed(&self) {}
    fn write_barrier(&mut self, target: WriteTarget);
}

pub struct NoBarrier;

impl Barrier for NoBarrier {
    fn flush(&mut self) {}
    fn write_barrier(&mut self, _target: WriteTarget) {
        unreachable!("write_barrier called on NoBarrier")
    }
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

    #[inline(always)]
    fn enqueue_node(&mut self, obj: ObjectReference) {
        if !*crate::IN_CONCURRENT_GC.lock() {
            return;
        }
        if compare_exchange_metadata::<E::VM>(
            &self.meta,
            obj,
            0b0,
            0b1,
            None,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            self.modbuf.push(obj);
            if self.modbuf.len() >= E::CAPACITY {
                self.flush();
            }
        }
    }
}

impl<E: ProcessEdgesWork> Barrier for ObjectRememberingBarrier<E> {
    #[cold]
    fn flush(&mut self) {
        if self.modbuf.is_empty() {
            return;
        }
        let mut modbuf = vec![];
        std::mem::swap(&mut modbuf, &mut self.modbuf);
        debug_assert!(
            !self.mmtk.scheduler.work_buckets[WorkBucketStage::Final].is_activated(),
            "{:?}",
            self as *const _
        );
        if !modbuf.is_empty() {
            self.mmtk.scheduler.work_buckets[WorkBucketStage::RefClosure]
                .add(ProcessModBuf::<E>::new(modbuf, self.meta));
        }
    }

    #[inline(always)]
    fn write_barrier(&mut self, target: WriteTarget) {
        // println!("write_barrier {:?}\n", target);
        match target {
            WriteTarget::Field { src, slot, val } => {
                if !*crate::IN_CONCURRENT_GC.lock() {
                    return;
                }
                let deleted = unsafe { slot.load::<ObjectReference>() };
                if deleted.is_null() {
                    return;
                }
                if !deleted.is_null() {
                    self.enqueue_node(deleted);
                }
            }
            WriteTarget::ArrayCopy { src, len, .. } => {
                unimplemented!();
            }
            WriteTarget::Clone {..} => {
                unimplemented!();
            }
        }
    }
}

pub struct FieldLoggingBarrier<E: ProcessEdgesWork> {
    mmtk: &'static MMTK<E::VM>,
    modbuf: Vec<Address>,
    /// The metadata used for log bit. Though this allows taking an arbitrary metadata spec,
    /// for this field, 0 means logged, and 1 means unlogged (the same as the vm::object_model::VMGlobalLogBitSpec).
    meta: MetadataSpec,
}

impl<E: ProcessEdgesWork> FieldLoggingBarrier<E> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<E::VM>, meta: MetadataSpec) -> Self {
        Self {
            mmtk,
            modbuf: vec![],
            meta,
        }
    }

    #[inline(always)]
    fn log_edge(&self, edge: Address) -> bool {
        loop {
            let old_value = load_metadata::<E::VM>(
                &self.meta,
                unsafe { edge.to_object_reference() },
                None,
                Some(Ordering::SeqCst),
            );
            if old_value == 1 {
                return false;
            }
            if compare_exchange_metadata::<E::VM>(
                &self.meta,
                unsafe { edge.to_object_reference() },
                0,
                1,
                None,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                return true;
            }
        }
    }

    #[inline(always)]
    fn enqueue_edge(&mut self, edge: Address) {
        if !*crate::IN_CONCURRENT_GC.lock() {
            return;
        }
        if self.log_edge(edge) {
            self.modbuf.push(edge);
            if self.modbuf.len() >= E::CAPACITY {
                self.flush();
            }
        }
    }
}

impl<E: ProcessEdgesWork> Barrier for FieldLoggingBarrier<E> {
    #[cold]
    fn flush(&mut self) {
        if self.modbuf.is_empty() {
            return;
        }
        let mut modbuf = vec![];
        std::mem::swap(&mut modbuf, &mut self.modbuf);
        debug_assert!(
            !self.mmtk.scheduler.work_buckets[WorkBucketStage::Final].is_activated(),
            "{:?}",
            self as *const _
        );
        self.mmtk.scheduler.work_buckets[WorkBucketStage::RefClosure].add(ProcessEdgeModBuf::<E>::new(modbuf, self.meta));
    }

    fn assert_is_flushed(&self) {
        assert!(self.modbuf.is_empty());
    }

    #[inline(always)]
    fn write_barrier(&mut self, target: WriteTarget) {
        match target {
            WriteTarget::Field { src, slot, val } => {
                if !*crate::IN_CONCURRENT_GC.lock() {
                    return;
                }
                self.enqueue_edge(slot);
            }
            WriteTarget::ArrayCopy { src, len, .. } => {
                for i in 0..len {
                    self.enqueue_edge(src + (i << 3));
                }
            }
            WriteTarget::Clone {..} => {}
        }
    }
}
