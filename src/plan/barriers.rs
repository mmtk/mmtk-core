//! Read/Write barrier implementations.

use atomic::Ordering;

use crate::scheduler::gc_work::*;
use crate::scheduler::WorkBucketStage;
use crate::util::metadata::{compare_exchange_metadata, MetadataSpec};
use crate::util::*;
use crate::vm::VMBinding;
use crate::MMTK;

/// BarrierSelector describes which barrier to use.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum BarrierSelector {
    NoBarrier,
    ObjectBarrier,
}

/// For field writes in HotSpot, we cannot always get the source object pointer and the field address\
#[derive(Debug)]
pub enum WriteTarget {
    Field(ObjectReference, Address, ObjectReference),
}

pub trait Barrier: 'static + Send {
    fn flush(&mut self);
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
        // if compare_exchange_metadata::<E::VM>(
        //     &self.meta,
        //     obj,
        //     0b1,
        //     0b0,
        //     None,
        //     Ordering::SeqCst,
        //     Ordering::SeqCst,
        // ) {
            self.modbuf.push(obj);
            if self.modbuf.len() >= E::CAPACITY {
                self.flush();
            }
        // }
    }
}

impl<E: ProcessEdgesWork> Barrier for ObjectRememberingBarrier<E> {
    #[cold]
    fn flush(&mut self) {
        println!("BARRIE FLUSH");
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
            WriteTarget::Field(obj, slot, val) => {
                // println!("{:?}.{:?} = {:?}", obj, slot, val);
                let deleted = unsafe { slot.load::<ObjectReference>() };
                if !deleted.is_null() {
                    self.enqueue_node(deleted);
                }
            }
        }
    }
}
