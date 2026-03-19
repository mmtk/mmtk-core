use atomic::Ordering;

use crate::plan::tracing::EdgeTracer;
use crate::plan::PlanTraceObject;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::{gc_work::*, GCWork, GCWorker, WorkBucketStage};
use crate::util::os::*;
use crate::util::ObjectReference;
use crate::vm::slot::{MemorySlice, Slot};
use crate::vm::*;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use super::global::GenerationalPlanExt;

pub struct GenNurseryEdgeTracer<
    VM: VMBinding,
    P: GenerationalPlanExt<VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    GenNurseryEdgeTracer<VM, P, KIND>
{
    pub(crate) fn new(plan: &'static P) -> Self {
        Self {
            plan,
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind> Clone
    for GenNurseryEdgeTracer<VM, P, KIND>
{
    fn clone(&self) -> Self {
        Self {
            plan: self.plan,
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    EdgeTracer for GenNurseryEdgeTracer<VM, P, KIND>
{
    type VM = VM;

    type ProcessSlotsWorkType = GenNurseryProcessSlots<VM, P, KIND>;
    type ScanObjectsWorkType = PlanScanObjects<Self, P>;

    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self {
            plan: mmtk.get_plan().downcast_ref().unwrap(),
            phantom_data: PhantomData,
        }
    }

    fn trace_object<Q: crate::ObjectQueue>(
        &mut self,
        worker: &mut GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        self.plan
            .trace_object_nursery::<_, KIND>(queue, object, worker)
    }

    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        _mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ScanObjectsWorkType {
        PlanScanObjects::new(self.plan, nodes, false, bucket)
    }
}

/// Process edges for a nursery GC. This type is provided if a generational plan does not use
/// [`crate::scheduler::gc_work::SFTProcessEdges`]. If a plan uses `SFTProcessEdges`,
/// it does not need to use this type.
pub struct GenNurseryProcessSlots<
    VM: VMBinding,
    P: GenerationalPlanExt<VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
    tracer: GenNurseryEdgeTracer<VM, P, KIND>,
    base: ProcessSlotsBase<VM>,
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ProcessSlotsWork for GenNurseryProcessSlots<VM, P, KIND>
{
    type VM = VM;
    type ScanObjectsWorkType = PlanScanObjects<GenNurseryEdgeTracer<VM, P, KIND>, P>;

    fn new(
        slots: Vec<SlotOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let base = ProcessSlotsBase::new(slots, roots, mmtk, bucket);
        let plan = base.plan().downcast_ref::<P>().unwrap();
        let tracer = GenNurseryEdgeTracer::new(plan);
        Self { plan, tracer, base }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        self.tracer
            .trace_object(self.worker(), object, &mut self.base.nodes)
    }

    fn process_slot(&mut self, slot: SlotOf<Self>) {
        let Some(object) = slot.load() else {
            // Skip slots that are not holding an object reference.
            return;
        };
        let new_object = self.trace_object(object);
        debug_assert!(!self.plan.is_object_in_nursery(new_object));
        // Note: If `object` is a mature object, `trace_object` will not call `space.trace_object`,
        // but will still return `object`.  In that case, we don't need to write it back.
        if new_object != object {
            slot.store(new_object);
        }
    }

    fn create_scan_work(&self, nodes: Vec<ObjectReference>) -> Option<Self::ScanObjectsWorkType> {
        Some(PlanScanObjects::new(self.plan, nodes, false, self.bucket))
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind> Deref
    for GenNurseryProcessSlots<VM, P, KIND>
{
    type Target = ProcessSlotsBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    DerefMut for GenNurseryProcessSlots<VM, P, KIND>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

/// The modbuf contains a list of objects in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet scans the recorded objects and forwards pointers if necessary.
pub struct ProcessModBuf<E: EdgeTracer> {
    modbuf: Vec<ObjectReference>,
    phantom: PhantomData<E>,
}

impl<E: EdgeTracer> ProcessModBuf<E> {
    pub fn new(modbuf: Vec<ObjectReference>) -> Self {
        debug_assert!(!modbuf.is_empty());
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<E: EdgeTracer> GCWork<E::VM> for ProcessModBuf<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // Process and scan modbuf only if the current GC is a nursery GC
        let gen = mmtk.get_plan().generational().unwrap();
        if gen.is_current_gc_nursery() {
            // Flip the per-object unlogged bits to "unlogged" state.
            for obj in &self.modbuf {
                debug_assert!(
                    !gen.is_object_in_nursery(*obj),
                    "{} was logged but is not mature. Dumping process memory maps:\n{}",
                    *obj,
                    OS::get_process_memory_maps().unwrap(),
                );
                <E::VM as VMBinding>::VMObjectModel::GLOBAL_LOG_BIT_SPEC.store_atomic::<E::VM, u8>(
                    *obj,
                    1,
                    None,
                    Ordering::SeqCst,
                );
            }
            // Scan objects in the modbuf and forward pointers
            let modbuf = std::mem::take(&mut self.modbuf);
            GCWork::do_work(
                &mut ScanObjects::<E>::new(modbuf, false, WorkBucketStage::Closure),
                worker,
                mmtk,
            )
        }
    }
}

/// The array-copy modbuf contains a list of array slices in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet forwards and updates each entry in the recorded slices.
pub struct ProcessRegionModBuf<E: EdgeTracer> {
    /// A list of `(start_address, bytes)` tuple.
    modbuf: Vec<<E::VM as VMBinding>::VMMemorySlice>,
    phantom: PhantomData<E>,
}

impl<E: EdgeTracer> ProcessRegionModBuf<E> {
    pub fn new(modbuf: Vec<<E::VM as VMBinding>::VMMemorySlice>) -> Self {
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<E: EdgeTracer> GCWork<E::VM> for ProcessRegionModBuf<E> {
    fn do_work(&mut self, worker: &mut GCWorker<E::VM>, mmtk: &'static MMTK<E::VM>) {
        // Scan modbuf only if the current GC is a nursery GC
        if mmtk
            .get_plan()
            .generational()
            .unwrap()
            .is_current_gc_nursery()
        {
            // Collect all the entries in all the slices
            let mut slots = vec![];
            for slice in &self.modbuf {
                for slot in slice.iter_slots() {
                    slots.push(slot);
                }
            }
            // Forward entries
            GCWork::do_work(
                &mut E::from_mmtk(mmtk).make_process_slots_work(
                    slots,
                    false,
                    mmtk,
                    WorkBucketStage::Closure,
                ),
                worker,
                mmtk,
            )
        }
    }
}
