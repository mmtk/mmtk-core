use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::scheduler::WorkBucketStage;
use crate::util::*;
use crate::MMTK;

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

#[derive(Default)]
pub struct ModBuffer {
    modified_nodes: Vec<ObjectReference>,
    modified_edges: Vec<Address>,
}

pub struct FieldRememberingBarrier<E: ProcessEdgesWork, S: Space<E::VM>> {
    mmtk: &'static MMTK<E::VM>,
    nursery: &'static S,
    mod_buffer: ModBuffer,
}

impl<E: ProcessEdgesWork, S: Space<E::VM>> FieldRememberingBarrier<E, S> {
    #[allow(unused)]
    pub fn new(mmtk: &'static MMTK<E::VM>, nursery: &'static S) -> Self {
        Self {
            mmtk,
            nursery,
            mod_buffer: ModBuffer::default(),
        }
    }

    fn enqueue_node(&mut self, obj: ObjectReference) {
        self.mod_buffer.modified_nodes.push(obj);
        if self.mod_buffer.modified_nodes.len() >= E::CAPACITY {
            self.flush();
        }
    }

    fn enqueue_edge(&mut self, slot: Address) {
        self.mod_buffer.modified_edges.push(slot);
        if self.mod_buffer.modified_edges.len() >= 512 {
            self.flush();
        }
    }
}

impl<E: ProcessEdgesWork, S: Space<E::VM>> Barrier for FieldRememberingBarrier<E, S> {
    fn flush(&mut self) {
        let mut modified_nodes = vec![];
        std::mem::swap(&mut modified_nodes, &mut self.mod_buffer.modified_nodes);
        let mut modified_edges = vec![];
        std::mem::swap(&mut modified_edges, &mut self.mod_buffer.modified_edges);
        debug_assert!(
            !self.mmtk.scheduler.work_buckets[WorkBucketStage::Final].is_activated(),
            "{:?}",
            self as *const _
        );
        self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure]
            .add(ProcessModBuf::<E>::new(modified_nodes, modified_edges));
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
