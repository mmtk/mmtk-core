use atomic::Ordering;

use crate::plan::tracing::TracePolicy;
use crate::plan::PlanTraceObject;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::{gc_work::*, GCWork, GCWorker, WorkBucketStage};
use crate::util::os::*;
use crate::util::ObjectReference;
use crate::vm::slot::MemorySlice;
use crate::vm::*;
use crate::MMTK;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use super::global::GenerationalPlanExt;

pub struct GenNurseryTracePolicy<
    VM: VMBinding,
    P: GenerationalPlanExt<VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind> Clone
    for GenNurseryTracePolicy<VM, P, KIND>
{
    fn clone(&self) -> Self {
        Self {
            plan: self.plan,
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    TracePolicy for GenNurseryTracePolicy<VM, P, KIND>
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

    fn may_move_objects() -> bool {
        true
    }

    fn is_concurrent() -> bool {
        false
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
    base: DefaultProcessSlots<GenNurseryTracePolicy<VM, P, KIND>>,
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ProcessSlotsWork for GenNurseryProcessSlots<VM, P, KIND>
{
    type VM = VM;
    type ScanObjectsWorkType = PlanScanObjects<GenNurseryTracePolicy<VM, P, KIND>, P>;

    fn new(
        slots: Vec<SlotOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let policy = GenNurseryTracePolicy::from_mmtk(mmtk);
        let base = DefaultProcessSlots::new(policy, slots, roots, bucket);
        Self { base }
    }

    fn trace_object(&mut self, _object: ObjectReference) -> ObjectReference {
        unimplemented!()
    }

    fn process_slot(&mut self, _slot: SlotOf<Self>) {
        unimplemented!()
    }

    fn create_scan_work(&self, _nodes: Vec<ObjectReference>) -> Option<Self::ScanObjectsWorkType> {
        unimplemented!()
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    GCWork<VM> for GenNurseryProcessSlots<VM, P, KIND>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        self.base.do_work(worker, mmtk);
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
pub struct ProcessModBuf<T: TracePolicy> {
    modbuf: Vec<ObjectReference>,
    phantom: PhantomData<T>,
}

impl<T: TracePolicy> ProcessModBuf<T> {
    pub fn new(modbuf: Vec<ObjectReference>) -> Self {
        debug_assert!(!modbuf.is_empty());
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<T: TracePolicy> GCWork<T::VM> for ProcessModBuf<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
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
                <T::VM as VMBinding>::VMObjectModel::GLOBAL_LOG_BIT_SPEC.store_atomic::<T::VM, u8>(
                    *obj,
                    1,
                    None,
                    Ordering::SeqCst,
                );
            }
            // Scan objects in the modbuf and forward pointers
            let modbuf = std::mem::take(&mut self.modbuf);
            GCWork::do_work(
                &mut ScanObjects::<T>::new(modbuf, false, WorkBucketStage::Closure),
                worker,
                mmtk,
            )
        }
    }
}

/// The array-copy modbuf contains a list of array slices in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet forwards and updates each entry in the recorded slices.
pub struct ProcessRegionModBuf<T: TracePolicy> {
    /// A list of `(start_address, bytes)` tuple.
    modbuf: Vec<<T::VM as VMBinding>::VMMemorySlice>,
    phantom: PhantomData<T>,
}

impl<T: TracePolicy> ProcessRegionModBuf<T> {
    pub fn new(modbuf: Vec<<T::VM as VMBinding>::VMMemorySlice>) -> Self {
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<T: TracePolicy> GCWork<T::VM> for ProcessRegionModBuf<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
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
                &mut T::from_mmtk(mmtk).make_process_slots_work(
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
