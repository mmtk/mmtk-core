//! The fundamental mechanism for performing a transitive closure over an object graph.

use std::mem;

use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::scheduler::{GCWorker, WorkBucketStage};
use crate::util::{Address, ObjectReference};
use crate::MMTK;

/// This trait is the fundamental mechanism for performing a
/// transitive closure over an object graph.
pub trait TransitiveClosure {
    // The signature of this function changes during the port
    // because the argument `ObjectReference source` is never used in the original version
    // See issue #5
    fn process_edge(&mut self, slot: Address);
    fn process_node(&mut self, object: ObjectReference);
}

impl<T: ProcessEdgesWork> TransitiveClosure for T {
    fn process_edge(&mut self, _slot: Address) {
        unreachable!();
    }
    #[inline]
    fn process_node(&mut self, object: ObjectReference) {
        ProcessEdgesWork::process_node(self, object);
    }
}

/// A transitive closure visitor to collect all the edges of an object.
pub struct ObjectsClosure<'a, E: ProcessEdgesWork> {
    mmtk: &'static MMTK<E::VM>,
    buffer: Vec<Address>,
    worker: &'a mut GCWorker<E::VM>,
}

impl<'a, E: ProcessEdgesWork> ObjectsClosure<'a, E> {
    pub fn new(
        mmtk: &'static MMTK<E::VM>,
        buffer: Vec<Address>,
        worker: &'a mut GCWorker<E::VM>,
    ) -> Self {
        Self {
            mmtk,
            buffer,
            worker,
        }
    }
}

impl<'a, E: ProcessEdgesWork> TransitiveClosure for ObjectsClosure<'a, E> {
    #[inline(always)]
    fn process_edge(&mut self, slot: Address) {
        if self.buffer.is_empty() {
            self.buffer.reserve(E::CAPACITY);
        }
        self.buffer.push(slot);
        if self.buffer.len() >= E::CAPACITY {
            let mut new_edges = Vec::new();
            mem::swap(&mut new_edges, &mut self.buffer);
            self.worker.add_work(
                WorkBucketStage::Closure,
                E::new(new_edges, false, self.mmtk),
            );
        }
    }
    fn process_node(&mut self, _object: ObjectReference) {
        unreachable!()
    }
}

impl<'a, E: ProcessEdgesWork> Drop for ObjectsClosure<'a, E> {
    #[inline(always)]
    fn drop(&mut self) {
        let mut new_edges = Vec::new();
        mem::swap(&mut new_edges, &mut self.buffer);
        self.worker.add_work(
            WorkBucketStage::Closure,
            E::new(new_edges, false, self.mmtk),
        );
    }
}

use crate::vm::VMBinding;
use crate::util::copy::CopySemantics;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::GCWork;

pub trait PlanTraceObject<VM: VMBinding> {
    fn trace_object<T: TransitiveClosure, const KIND: TraceKind>(&self, trace: &mut T, object: ObjectReference, worker: &mut GCWorker<VM>) -> ObjectReference;
    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(&'static self, nodes: Vec<ObjectReference>) -> Box<dyn GCWork<VM>>;
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}

pub trait PolicyTraceObject<VM: VMBinding> {
    fn trace_object<T: TransitiveClosure, const KIND: TraceKind>(&self, trace: &mut T, object: ObjectReference, copy: Option<CopySemantics>, worker: &mut GCWorker<VM>) -> ObjectReference;
    #[inline(always)]
    fn create_scan_work<E: ProcessEdgesWork<VM = VM>>(
        &'static self,
        nodes: Vec<ObjectReference>,
    ) -> Box<dyn GCWork<VM>> {
        Box::new(crate::scheduler::gc_work::ScanObjects::<E>::new(
            nodes, false,
        ))
    }
    fn may_move_objects<const KIND: TraceKind>() -> bool;
}
