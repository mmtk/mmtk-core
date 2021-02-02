use atomic_traits::fetch::Add;

use crate::policy::space::Space;
use crate::scheduler::gc_works::*;
use crate::util::constants::*;
use crate::util::metadata::*;
use crate::util::*;
use crate::MMTK;

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
}

impl<E: ProcessEdgesWork, S: Space<E::VM>> ObjectRememberingBarrier<E, S> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<E::VM>, nursery: &'static S) -> Self {
        Self {
            mmtk,
            nursery,
            modbuf: vec![],
        }
    }

    #[inline(always)]
    fn enqueue_node(&mut self, obj: ObjectReference) {
        if (BARRIER_COUNTER) {
            BARRIER_FAST_COUNT.fetch_add(1, atomic::Ordering::SeqCst);
        }
        if BitsReference::of(obj.to_address(), LOG_BYTES_IN_WORD, 0).attempt(0b1, 0b0) {
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
            !self.mmtk.scheduler.final_stage.is_activated(),
            "{:?}",
            self as *const _
        );
        if modbuf.len() != 0 {
            self.mmtk
                .scheduler
                .closure_stage
                .add(ProcessModBuf::<E>::new(modbuf));
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
