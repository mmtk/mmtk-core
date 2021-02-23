use atomic_traits::fetch::Add;

use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::constants::*;
use crate::util::metadata::*;
use crate::scheduler::WorkBucketStage;
use crate::util::*;
use crate::MMTK;
use crate::util::side_metadata::*;

use super::mutator_context::{BARRIER_COUNTER, BARRIER_FAST_COUNT, BARRIER_SLOW_COUNT};

/// For field writes in HotSpot, we cannot always get the source object pointer and the field address
pub enum WriteTarget {
    Object(ObjectReference),
    Slot(Address),
}

pub trait Barrier: 'static + Send + Sync {
    fn flush(&mut self);
    fn post_write_barrier(&mut self, target: WriteTarget);
}

pub struct NoBarrier;

impl Barrier for NoBarrier {
    fn flush(&mut self) {}
    fn post_write_barrier(&mut self, _target: WriteTarget) {}
}

pub struct ObjectRememberingBarrier<E: ProcessEdgesWork, S: Space<E::VM>> {
    mmtk: &'static MMTK<E::VM>,
    nursery: &'static S,
    modbuf: Vec<ObjectReference>,
    meta: SideMetadataSpec,
}

impl<E: ProcessEdgesWork, S: Space<E::VM>> ObjectRememberingBarrier<E, S> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<E::VM>, nursery: &'static S, meta: SideMetadataSpec) -> Self {
        Self {
            mmtk,
            nursery,
            modbuf: vec![],
            meta,
        }
    }

    #[inline(always)]
    fn enqueue_node(&mut self, obj: ObjectReference) {
        if (BARRIER_COUNTER) {
            BARRIER_FAST_COUNT.fetch_add(1, atomic::Ordering::SeqCst);
        }
        if compare_exchange_atomic(self.meta, obj.to_address(), 0b1, 0b0) {
            // store_atomic(self.meta, obj.to_address(), 0b1);
            if (BARRIER_COUNTER) {
                BARRIER_SLOW_COUNT.fetch_add(1, atomic::Ordering::SeqCst);
            }
            self.modbuf.push(obj);
            if self.modbuf.len() >= E::CAPACITY {
                self.flush();
            }
        }
    }
}

impl<E: ProcessEdgesWork, S: Space<E::VM>> Barrier for ObjectRememberingBarrier<E, S> {
    #[cold]
    fn flush(&mut self) {
        let mut modbuf = vec![];
        std::mem::swap(&mut modbuf, &mut self.modbuf);
        debug_assert!(
            !self.mmtk.scheduler.work_buckets[WorkBucketStage::Final].is_activated(),
            "{:?}",
            self as *const _
        );
        if modbuf.len() != 0 {
            self.mmtk
                .scheduler
                .work_buckets[WorkBucketStage::Closure]
                .add(ProcessModBuf::<E>::new(modbuf, self.meta));
        }
    }

    #[inline(always)]
    fn post_write_barrier(&mut self, target: WriteTarget) {
        // unreachable!();
        match target {
            WriteTarget::Object(obj) => {
                self.enqueue_node(obj);
            }
            _ => unreachable!(),
        }
    }
}
