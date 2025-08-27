use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::plan::PlanTraceObject;
use crate::plan::VectorQueue;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::gc_work::{ScanObjects, SlotOf};
use crate::util::ObjectReference;
use crate::vm::slot::Slot;
use crate::{
    plan::ObjectQueue,
    scheduler::{gc_work::ProcessEdgesBase, GCWork, GCWorker, ProcessEdgesWork, WorkBucketStage},
    vm::*,
    MMTK,
};
use std::ops::{Deref, DerefMut};

pub struct ConcurrentTraceObjects<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
    // objects to mark and scan
    objects: Option<Vec<ObjectReference>>,
    // recursively generated objects
    next_objects: VectorQueue<ObjectReference>,
    worker: *mut GCWorker<VM>,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ConcurrentTraceObjects<VM, P, KIND>
{
    const SATB_BUFFER_SIZE: usize = 8192;

    pub fn new(objects: Vec<ObjectReference>, mmtk: &'static MMTK<VM>) -> Self {
        let plan = mmtk.get_plan().downcast_ref::<P>().unwrap();

        Self {
            plan,
            objects: Some(objects),
            next_objects: VectorQueue::default(),
            worker: std::ptr::null_mut(),
        }
    }

    pub fn worker(&self) -> &'static mut GCWorker<VM> {
        debug_assert_ne!(self.worker, std::ptr::null_mut());
        unsafe { &mut *self.worker }
    }

    #[cold]
    fn flush(&mut self) {
        if !self.next_objects.is_empty() {
            let objects = self.next_objects.take();
            let worker = self.worker();
            let w = Self::new(objects, worker.mmtk);
            worker.add_work(WorkBucketStage::Concurrent, w);
        }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        let new_object = self
            .plan
            .trace_object::<Self, KIND>(self, object, self.worker());
        // No copying should happen.
        debug_assert_eq!(object, new_object);
        object
    }

    fn trace_objects(&mut self, objects: &[ObjectReference]) {
        for o in objects.iter() {
            self.trace_object(*o);
        }
    }

    fn scan_and_enqueue(&mut self, object: ObjectReference) {
        crate::plan::tracing::SlotIterator::<VM, _>::iterate_fields(
            object,
            self.worker().tls.0,
            |s| {
                let Some(t) = s.load() else {
                    return;
                };

                self.next_objects.push(t);
                if self.next_objects.len() > Self::SATB_BUFFER_SIZE {
                    self.flush();
                }
            },
        );
        self.plan.post_scan_object(object);
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ObjectQueue for ConcurrentTraceObjects<VM, P, KIND>
{
    fn enqueue(&mut self, object: ObjectReference) {
        debug_assert!(
            object.to_raw_address().is_mapped(),
            "Invalid obj {:?}: address is not mapped",
            object
        );
        self.scan_and_enqueue(object);
    }
}

unsafe impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    Send for ConcurrentTraceObjects<VM, P, KIND>
{
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    GCWork<VM> for ConcurrentTraceObjects<VM, P, KIND>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.worker = worker;
        let mut num_objects = 0;
        let mut num_next_objects = 0;
        let mut iterations = 0;
        // mark objects
        if let Some(objects) = self.objects.take() {
            self.trace_objects(&objects);
            num_objects = objects.len();
        }
        let pause_opt = self.plan.current_pause();
        if pause_opt == Some(Pause::FinalMark) || pause_opt.is_none() {
            while !self.next_objects.is_empty() {
                let pause_opt = self.plan.current_pause();
                if !(pause_opt == Some(Pause::FinalMark) || pause_opt.is_none()) {
                    break;
                }
                let next_objects = self.next_objects.take();
                self.trace_objects(&next_objects);
                num_next_objects += next_objects.len();
                iterations += 1;
            }
        }
        probe!(
            mmtk,
            concurrent_trace_objects,
            num_objects,
            num_next_objects,
            iterations
        );
        self.flush();
    }
}

pub struct ProcessModBufSATB<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    nodes: Option<Vec<ObjectReference>>,
    _p: std::marker::PhantomData<(VM, P)>,
}

unsafe impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    Send for ProcessModBufSATB<VM, P, KIND>
{
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ProcessModBufSATB<VM, P, KIND>
{
    pub fn new(nodes: Vec<ObjectReference>) -> Self {
        Self {
            nodes: Some(nodes),
            _p: std::marker::PhantomData,
        }
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    GCWork<VM> for ProcessModBufSATB<VM, P, KIND>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let mut w = if let Some(nodes) = self.nodes.take() {
            if nodes.is_empty() {
                return;
            }

            ConcurrentTraceObjects::<VM, P, KIND>::new(nodes, mmtk)
        } else {
            return;
        };
        GCWork::do_work(&mut w, worker, mmtk);
    }
}

pub struct ProcessRootSlots<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    base: ProcessEdgesBase<VM>,
    _p: std::marker::PhantomData<P>,
}

unsafe impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    Send for ProcessRootSlots<VM, P, KIND>
{
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ProcessRootSlots<VM, P, KIND>
{
    fn create_and_schedule_concurrent_trace_objects_work(&self, objects: Vec<ObjectReference>) {
        let worker = self.worker();
        let mmtk = self.mmtk();
        let w = ConcurrentTraceObjects::<VM, P, KIND>::new(objects.clone(), mmtk);

        worker.scheduler().work_buckets[WorkBucketStage::Concurrent].add_no_notify(w);
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ProcessEdgesWork for ProcessRootSlots<VM, P, KIND>
{
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;
    const OVERWRITE_REFERENCE: bool = false;
    const SCAN_OBJECTS_IMMEDIATELY: bool = true;

    fn new(
        slots: Vec<SlotOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        debug_assert!(roots);
        let base = ProcessEdgesBase::new(slots, roots, mmtk, bucket);
        Self {
            base,
            _p: std::marker::PhantomData,
        }
    }

    fn flush(&mut self) {}

    fn trace_object(&mut self, _object: ObjectReference) -> ObjectReference {
        unreachable!()
    }

    fn process_slots(&mut self) {
        let pause = self
            .base
            .plan()
            .concurrent()
            .unwrap()
            .current_pause()
            .unwrap();
        // No need to scan roots in the final mark
        if pause == Pause::FinalMark {
            return;
        }
        debug_assert_eq!(pause, Pause::InitialMark);
        let mut root_objects = Vec::with_capacity(Self::CAPACITY);
        if !self.slots.is_empty() {
            let slots = std::mem::take(&mut self.slots);
            for slot in slots {
                if let Some(object) = slot.load() {
                    root_objects.push(object);
                    if root_objects.len() == Self::CAPACITY {
                        self.create_and_schedule_concurrent_trace_objects_work(
                            root_objects.drain(..).collect(),
                        );
                    }
                }
            }
            if !root_objects.is_empty() {
                self.create_and_schedule_concurrent_trace_objects_work(root_objects);
            }
        }
    }

    fn create_scan_work(&self, _nodes: Vec<ObjectReference>) -> Self::ScanObjectsWorkType {
        unimplemented!()
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind> Deref
    for ProcessRootSlots<VM, P, KIND>
{
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    DerefMut for ProcessRootSlots<VM, P, KIND>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
