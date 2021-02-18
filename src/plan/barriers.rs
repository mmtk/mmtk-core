use atomic_traits::fetch::Add;

use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::constants::*;
use crate::scheduler::WorkBucketStage;
use crate::util::*;
use crate::MMTK;
use crate::util::side_metadata::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Copy, Clone, Debug)]
pub enum BarrierSelector {
    NoBarrier,
    ObjectBarrier,
}

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
        if ENABLE_BARRIER_COUNTER {
            BARRIER_COUNTER.total.fetch_add(1, atomic::Ordering::SeqCst);
        }
        if compare_exchange_atomic(self.meta, obj.to_address(), 0b1, 0b0) {
            if ENABLE_BARRIER_COUNTER {
                BARRIER_COUNTER.slow.fetch_add(1, atomic::Ordering::SeqCst);
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
        match target {
            WriteTarget::Object(obj) => {
                self.enqueue_node(obj);
            }
            _ => unreachable!(),
        }
    }
}

/// Note: Please also disable vm-binding's barrier fast-path.
pub const ENABLE_BARRIER_COUNTER: bool = false;

pub static BARRIER_COUNTER: BarrierCounter = BarrierCounter {
    total: AtomicUsize::new(0),
    slow: AtomicUsize::new(0),
};

pub struct BarrierCounter {
    pub total: AtomicUsize,
    pub slow: AtomicUsize,
}

pub struct BarrierCounterResults {
    pub total: f64,
    pub slow: f64,
    pub take_rate: f64,
}

impl BarrierCounter {
    pub fn reset(&self) {
        self.total.store(0, Ordering::SeqCst);
        self.slow.store(0, Ordering::SeqCst);
    }

    pub fn get_results(&self) -> BarrierCounterResults {
        let total = self.total.load(Ordering::SeqCst) as f64;
        let slow = self.slow.load(Ordering::SeqCst) as f64;
        BarrierCounterResults {
            total, slow,
            take_rate: slow / total,
        }
    }
}
