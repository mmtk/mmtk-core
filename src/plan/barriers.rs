use crate::util::*;
#[cfg(feature="copyspace")]
use crate::policy::space::Space;
#[cfg(feature="copyspace")]
use crate::policy::copyspace::CopySpace;
#[cfg(feature="copyspace")]
use crate::scheduler::gc_works::*;
#[cfg(feature="copyspace")]
use crate::MMTK;

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

#[cfg(feature="copyspace")]
pub struct FieldRememberingBarrier<E: ProcessEdgesWork> {
    mmtk: &'static MMTK<E::VM>,
    nursery: &'static CopySpace<E::VM>,
    modbuf: Box<(Vec<ObjectReference>, Vec<Address>)>,
}

#[cfg(feature="copyspace")]
impl <E: ProcessEdgesWork> FieldRememberingBarrier<E> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<E::VM>, nursery: &'static CopySpace<E::VM>) -> Self {
        Self {
            mmtk, nursery,
            modbuf: box Default::default(),
        }
    }

    fn enqueue_node(&mut self, obj: ObjectReference) {
        self.modbuf.0.push(obj);
        if self.modbuf.0.len() >= 512 {
            self.flush();
        }
    }

    fn enqueue_edge(&mut self, slot: Address) {
        self.modbuf.1.push(slot);
        if self.modbuf.1.len() >= 512 {
            self.flush();
        }
    }
}

#[cfg(feature="copyspace")]
impl <E: ProcessEdgesWork> Barrier for FieldRememberingBarrier<E> {
    fn flush(&mut self) {
        let mut modified_nodes = vec![];
        std::mem::swap(&mut modified_nodes, &mut self.modbuf.0);
        let mut modified_edges = vec![];
        std::mem::swap(&mut modified_edges, &mut self.modbuf.1);
        debug_assert!(!self.mmtk.scheduler.final_stage.is_activated(), "{:?}", self as *const _);
        self.mmtk.scheduler.closure_stage.add(ProcessModBuf::<E>::new(modified_nodes, modified_edges));
    }
    fn post_write_barrier(&mut self, target: WriteTarget) {
        match target {
            WriteTarget::Object(obj) => {
                if !self.nursery.in_space(obj) {
                    self.enqueue_node(obj);
                }
            }
            WriteTarget::Slot(slot) => {
                if !self.nursery.address_in_space(slot) {
                    self.enqueue_edge(slot);
                }
            }
        }
    }
}