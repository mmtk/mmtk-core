use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::plan::tracing::TracePolicy;
use crate::plan::PlanTraceObject;
use crate::plan::VectorObjectQueue;
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
use std::collections::VecDeque;
use std::marker::PhantomData;

pub struct ConcurrentTraceObjects<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    policy: ConcurrentRootTracePolicy<VM, P, KIND>,
    /// initial objects to mark and scan
    initial_objects: Vec<ObjectReference>,
    /// `true` if the `initial_objects` are already marked.
    already_marked: bool,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ConcurrentTraceObjects<VM, P, KIND>
{
    const SATB_BUFFER_SIZE: usize = 8192;
    const CONCURRENT_TRACE_OVERFLOW: usize = Self::SATB_BUFFER_SIZE * 2;

    pub fn new(
        policy: ConcurrentRootTracePolicy<VM, P, KIND>,
        initial_objects: Vec<ObjectReference>,
        already_marked: bool,
    ) -> Self {
        Self {
            policy,
            initial_objects,
            already_marked,
        }
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
        let tls = worker.tls;

        // These are initial objects.  They may not have been marked.
        let initial_objects = std::mem::take(&mut self.initial_objects);
        let num_initial_objects = initial_objects.len();
        let mut num_queued_objects = 0;

        // This queue contains marked but not scanned objects.
        let mut queue = VecDeque::new();
        if self.already_marked {
            // The initial objects are already marked.  Put them in the queue.
            queue.extend(initial_objects);
        } else {
            // We scan each object and only enqueue newly visited objects.
            for object in initial_objects {
                self.policy
                    .trace_object(worker, object, &mut |enqueued_object| {
                        debug_assert_eq!(enqueued_object, object);
                        queue.push_back(enqueued_object);
                        num_queued_objects += 1;
                    });
            }
        }

        // Loop until the queue is drained.
        while let Some(object) = queue.pop_back() {
            if VM::VMScanning::support_slot_enqueuing(tls, object) {
                VM::VMScanning::scan_object(tls, object, &mut |slot: VM::VMSlot| {
                    if let Some(child) = slot.load() {
                        let new_child =
                            self.policy
                                .trace_object(worker, child, &mut |enqueued_child| {
                                    debug_assert_eq!(enqueued_child, child);
                                    queue.push_back(enqueued_child);
                                    num_queued_objects += 1;
                                });
                        debug_assert_eq!(new_child, child);
                    }
                });
            } else {
                VM::VMScanning::scan_object_and_trace_edges(tls, object, &mut |child| {
                    let new_child =
                        self.policy
                            .trace_object(worker, child, &mut |enqueued_child| {
                                debug_assert_eq!(enqueued_child, child);
                                queue.push_back(enqueued_child);
                                num_queued_objects += 1;
                            });
                    debug_assert_eq!(new_child, child);
                    new_child
                });
            }
            self.policy.post_scan_object(object);

            if queue.len() >= Self::CONCURRENT_TRACE_OVERFLOW {
                let offloaded_objects = queue.drain(..Self::SATB_BUFFER_SIZE).collect();
                let w = Self::new(self.policy.clone(), offloaded_objects, true);
                worker.add_work(WorkBucketStage::Concurrent, w);
            }
        }

        probe!(
            mmtk,
            concurrent_trace_objects,
            num_initial_objects,
            num_queued_objects
        );
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

            ConcurrentTraceObjects::<VM, P, KIND>::new(
                ConcurrentRootTracePolicy::from_mmtk(mmtk),
                nodes,
                false, // These objects are not marked, yet.
            )
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

    fn post_scan_object(&mut self, object: ObjectReference) {
        self.plan.post_scan_object(object);
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
        _mmtk: &'static MMTK<Self::VM>,
        _bucket: WorkBucketStage,
    ) -> Self::ScanObjectsWorkType {
        // This is called by root scanning, so
        ConcurrentTraceObjects::new(self.clone(), nodes, false)
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

        let mut queue = VectorObjectQueue::new();
        let flush = |queue: &mut VectorObjectQueue| {
            let objects = queue.take();
            let w = ConcurrentTraceObjects::<VM, P, KIND>::new(
                ConcurrentRootTracePolicy::from_mmtk(mmtk),
                objects,
                false, // These objects are not marked, yet.
            );
            worker.scheduler().work_buckets[WorkBucketStage::Concurrent].add_no_notify(w);
        };

        for slot in self.slots.iter() {
            if let Some(object) = slot.load() {
                queue.push(object);
                if queue.len() == EDGES_WORK_BUFFER_SIZE {
                    flush(&mut queue);
                }
            }
        }
        if !queue.is_empty() {
            flush(&mut queue);
        }
    }
}
