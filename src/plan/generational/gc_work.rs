use atomic::Ordering;

use crate::plan::PlanTraceObject;
use crate::scheduler::{gc_work::*, GCWork, GCWorker, WorkBucketStage};
use crate::util::ObjectReference;
use crate::vm::edge_shape::{Edge, MemorySlice};
use crate::vm::*;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use super::global::GenerationalPlanExt;

/// Process edges for a nursery GC. This type is provided if a generational plan does not use
/// [`crate::scheduler::gc_work::SFTProcessEdges`]. If a plan uses `SFTProcessEdges`,
/// it does not need to use this type.
pub struct GenNurseryProcessEdges<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>> {
    plan: &'static P,
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>> ProcessEdgesWork
    for GenNurseryProcessEdges<VM, P>
{
    type VM = VM;
    type ScanObjectsWorkType = PlanScanObjects<Self, P>;

    fn new(
        edges: Vec<EdgeOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let base = ProcessEdgesBase::new(edges, roots, mmtk, bucket);
        let plan = base.plan().downcast_ref().unwrap();
        Self { plan, base }
    }
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        debug_assert!(!object.is_null());

        // We cannot borrow `self` twice in a call, so we extract `worker` as a local variable.
        let worker = self.worker();
        self.plan
            .trace_object_nursery(&mut self.base.nodes, object, worker)
    }
    fn process_edge(&mut self, slot: EdgeOf<Self>) {
        let object = slot.load();
        if object.is_null() {
            return;
        }
        let new_object = self.trace_object(object);
        debug_assert!(!self.plan.is_object_in_nursery(new_object));
        // Note: If `object` is a mature object, `trace_object` will not call `space.trace_object`,
        // but will still return `object`.  In that case, we don't need to write it back.
        if new_object != object {
            slot.store(new_object);
        }
    }

    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        roots: bool,
    ) -> Self::ScanObjectsWorkType {
        PlanScanObjects::new(self.plan, nodes, false, roots, self.bucket)
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>> Deref
    for GenNurseryProcessEdges<VM, P>
{
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>> DerefMut
    for GenNurseryProcessEdges<VM, P>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

/// The modbuf contains a list of objects in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet scans the recorded objects and forwards pointers if necessary.
pub struct ProcessModBuf<E: ProcessEdgesWork> {
    modbuf: Vec<ObjectReference>,
    phantom: PhantomData<E>,
}

impl<E: ProcessEdgesWork> ProcessModBuf<E> {
    pub fn new(modbuf: Vec<ObjectReference>) -> Self {
        debug_assert!(!modbuf.is_empty());
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessModBuf<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // Flip the per-object unlogged bits to "unlogged" state.
        for obj in &self.modbuf {
            <E::VM as VMBinding>::VMObjectModel::GLOBAL_LOG_BIT_SPEC.store_atomic::<E::VM, u8>(
                *obj,
                1,
                None,
                Ordering::SeqCst,
            );
        }
        // scan modbuf only if the current GC is a nursery GC
        if mmtk
            .get_plan()
            .generational()
            .unwrap()
            .is_current_gc_nursery()
        {
            // Scan objects in the modbuf and forward pointers
            let modbuf = std::mem::take(&mut self.modbuf);
            GCWork::do_work(
                &mut ScanObjects::<E>::new(modbuf, false, false, WorkBucketStage::Closure),
                worker,
                mmtk,
            )
        }
    }
}

/// The array-copy modbuf contains a list of array slices in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet forwards and updates each entry in the recorded slices.
pub struct ProcessRegionModBuf<E: ProcessEdgesWork> {
    /// A list of `(start_address, bytes)` tuple.
    modbuf: Vec<<E::VM as VMBinding>::VMMemorySlice>,
    phantom: PhantomData<E>,
}

impl<E: ProcessEdgesWork> ProcessRegionModBuf<E> {
    pub fn new(modbuf: Vec<<E::VM as VMBinding>::VMMemorySlice>) -> Self {
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<E: ProcessEdgesWork> GCWork<E::VM> for ProcessRegionModBuf<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // Scan modbuf only if the current GC is a nursery GC
        if mmtk
            .get_plan()
            .generational()
            .unwrap()
            .is_current_gc_nursery()
        {
            // Collect all the entries in all the slices
            let mut edges = vec![];
            for slice in &self.modbuf {
                for edge in slice.iter_edges() {
                    edges.push(edge);
                }
            }
            // Forward entries
            GCWork::do_work(
                &mut E::new(edges, false, mmtk, WorkBucketStage::Closure),
                worker,
                mmtk,
            )
        }
    }
}
