use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::plan::tracing::TracePolicy;
use crate::plan::PlanTraceObject;
use crate::plan::VectorQueue;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::EDGES_WORK_BUFFER_SIZE;
use crate::util::ObjectReference;
use crate::vm::slot::Slot;
use crate::{
    plan::ObjectQueue,
    scheduler::{GCWork, GCWorker, WorkBucketStage},
    vm::*,
    MMTK,
};
use std::marker::PhantomData;

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
        crate::plan::tracing::SlotIterator::<VM>::iterate_fields(
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

pub struct ConcurrentRootTracePolicy<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind> Clone
    for ConcurrentRootTracePolicy<VM, P, KIND>
{
    fn clone(&self) -> Self {
        Self { plan: self.plan }
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    TracePolicy for ConcurrentRootTracePolicy<VM, P, KIND>
{
    type VM = VM;

    type ProcessSlotsWorkType = ProcessRootSlots<VM, P, KIND>;
    type ScanObjectsWorkType = ConcurrentTraceObjects<VM, P, KIND>;

    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self {
        let plan = mmtk.get_plan().downcast_ref::<P>().unwrap();
        Self { plan }
    }

    fn trace_object<Q: ObjectQueue>(
        &mut self,
        worker: &mut GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        self.plan.trace_object::<Q, KIND>(queue, object, worker)
    }

    fn make_process_slots_work(
        &self,
        slots: Vec<<Self::VM as VMBinding>::VMSlot>,
        roots: bool,
        mmtk: &'static MMTK<Self::VM>,
        bucket: WorkBucketStage,
    ) -> Self::ProcessSlotsWorkType {
        ProcessRootSlots::new(slots, roots, mmtk, bucket)
    }

    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        mmtk: &'static MMTK<Self::VM>,
        _bucket: WorkBucketStage,
    ) -> Self::ScanObjectsWorkType {
        ConcurrentTraceObjects::<VM, P, KIND>::new(nodes, mmtk)
    }

    fn may_move_objects() -> bool {
        // Concurrent marking never moves objects.
        false
    }

    fn is_concurrent() -> bool {
        true
    }
}

pub struct ProcessRootSlots<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    slots: Vec<VM::VMSlot>,
    phantom_data: PhantomData<P>,
}

unsafe impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    Send for ProcessRootSlots<VM, P, KIND>
{
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ProcessRootSlots<VM, P, KIND>
{
    fn create_and_schedule_concurrent_trace_objects_work(
        &self,
        worker: &mut GCWorker<VM>,
        objects: Vec<ObjectReference>,
    ) {
        let mmtk = worker.mmtk;
        let w = ConcurrentTraceObjects::<VM, P, KIND>::new(objects.clone(), mmtk);

        worker.scheduler().work_buckets[WorkBucketStage::Concurrent].add_no_notify(w);
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ProcessRootSlots<VM, P, KIND>
{
    fn new(
        slots: Vec<VM::VMSlot>,
        roots: bool,
        _mmtk: &'static MMTK<VM>,
        _bucket: WorkBucketStage,
    ) -> Self {
        debug_assert!(roots);
        Self {
            slots,
            phantom_data: PhantomData,
        }
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    GCWork<VM> for ProcessRootSlots<VM, P, KIND>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let pause = mmtk
            .get_plan()
            .concurrent()
            .unwrap()
            .current_pause()
            .unwrap();
        // No need to scan roots in the final mark
        if pause == Pause::FinalMark {
            return;
        }
        debug_assert_eq!(pause, Pause::InitialMark);
        let mut root_objects = Vec::with_capacity(EDGES_WORK_BUFFER_SIZE);
        if !self.slots.is_empty() {
            let slots = std::mem::take(&mut self.slots);
            for slot in slots {
                if let Some(object) = slot.load() {
                    root_objects.push(object);
                    if root_objects.len() == EDGES_WORK_BUFFER_SIZE {
                        let mut buffer = Vec::with_capacity(EDGES_WORK_BUFFER_SIZE);
                        std::mem::swap(&mut buffer, &mut root_objects);
                        self.create_and_schedule_concurrent_trace_objects_work(worker, buffer);
                    }
                }
            }
            if !root_objects.is_empty() {
                self.create_and_schedule_concurrent_trace_objects_work(worker, root_objects);
            }
        }
    }
}
