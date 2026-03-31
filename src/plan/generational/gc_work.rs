use atomic::Ordering;

use crate::plan::tracing::Trace;
use crate::plan::PlanTraceObject;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::{gc_work::*, GCWork, GCWorker, WorkBucketStage};
use crate::util::os::*;
use crate::util::ObjectReference;
use crate::vm::slot::MemorySlice;
use crate::vm::*;
use crate::MMTK;
use std::marker::PhantomData;

use super::global::GenerationalPlanExt;

pub struct GenNurseryTrace<
    VM: VMBinding,
    P: GenerationalPlanExt<VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
    phantom_data: PhantomData<VM>,
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind> Clone
    for GenNurseryTrace<VM, P, KIND>
{
    fn clone(&self) -> Self {
        Self {
            plan: self.plan,
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding, P: GenerationalPlanExt<VM> + PlanTraceObject<VM>, const KIND: TraceKind> Trace
    for GenNurseryTrace<VM, P, KIND>
{
    type VM = VM;

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

    fn post_scan_object(&mut self, object: ObjectReference) {
        self.plan.post_scan_object(object);
    }

    fn may_move_objects() -> bool {
        true
    }
}

/// The modbuf contains a list of objects in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet scans the recorded objects and forwards pointers if necessary.
pub struct ProcessModBuf<T: Trace> {
    modbuf: Vec<ObjectReference>,
    phantom: PhantomData<T>,
}

impl<T: Trace> ProcessModBuf<T> {
    pub fn new(modbuf: Vec<ObjectReference>) -> Self {
        debug_assert!(!modbuf.is_empty());
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<T: Trace> GCWork<T::VM> for ProcessModBuf<T> {
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
                &mut TracingProcessNodes::<T>::new(
                    T::from_mmtk(mmtk),
                    modbuf,
                    WorkBucketStage::Closure,
                ),
                worker,
                mmtk,
            )
        }
    }
}

/// The array-copy modbuf contains a list of array slices in mature space(s) that
/// may contain pointers to the nursery space.
/// This work packet forwards and updates each entry in the recorded slices.
pub struct ProcessRegionModBuf<T: Trace> {
    /// A list of `(start_address, bytes)` tuple.
    modbuf: Vec<<T::VM as VMBinding>::VMMemorySlice>,
    phantom: PhantomData<T>,
}

impl<T: Trace> ProcessRegionModBuf<T> {
    pub fn new(modbuf: Vec<<T::VM as VMBinding>::VMMemorySlice>) -> Self {
        Self {
            modbuf,
            phantom: PhantomData,
        }
    }
}

impl<T: Trace> GCWork<T::VM> for ProcessRegionModBuf<T> {
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
                &mut TracingProcessSlots::new(
                    T::from_mmtk(mmtk),
                    slots,
                    false,
                    WorkBucketStage::Closure,
                ),
                worker,
                mmtk,
            )
        }
    }
}
