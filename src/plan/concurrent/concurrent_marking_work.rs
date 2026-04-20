use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::Pause;
use crate::plan::tracing::Trace;
use crate::plan::PlanTraceObject;
use crate::policy::gc_work::TraceKind;
use crate::scheduler::gc_work::RootsKind;
use crate::util::{scanning_helper, ObjectReference};
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
    /// initial objects to mark and scan
    initial_objects: Vec<ObjectReference>,
    /// `true` if the `initial_objects` are already marked.
    already_marked: bool,
    phantom_data: PhantomData<(VM, P)>,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ConcurrentTraceObjects<VM, P, KIND>
{
    const SATB_BUFFER_SIZE: usize = 8192;
    const CONCURRENT_TRACE_OVERFLOW: usize = Self::SATB_BUFFER_SIZE * 2;

    pub fn new(initial_objects: Vec<ObjectReference>, already_marked: bool) -> Self {
        Self {
            initial_objects,
            already_marked,
            phantom_data: PhantomData,
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
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let tls = worker.tls;
        let mut trace = ConcurrentMarkingTrace::<VM, P, KIND>::from_mmtk(mmtk);

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
                trace.trace_object(worker, object, &mut |enqueued_object| {
                    debug_assert_eq!(enqueued_object, object);
                    queue.push_back(enqueued_object);
                    num_queued_objects += 1;
                });
            }
        }

        // Loop until the queue is drained.
        while let Some(object) = queue.pop_back() {
            scanning_helper::visit_children_non_moving::<VM>(tls, object, &mut |child| {
                trace.trace_object(worker, child, &mut |enqueued_child| {
                    debug_assert_eq!(enqueued_child, child);
                    queue.push_back(enqueued_child);
                    num_queued_objects += 1;
                })
            });
            trace.post_scan_object(object);

            if queue.len() >= Self::CONCURRENT_TRACE_OVERFLOW {
                let offloaded_objects = queue.drain(..Self::SATB_BUFFER_SIZE).collect();
                let w = Self::new(offloaded_objects, true);
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
                nodes, false, // These objects are not marked, yet.
            )
        } else {
            return;
        };
        GCWork::do_work(&mut w, worker, mmtk);
    }
}

pub struct ConcurrentMarkingTrace<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    plan: &'static P,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind> Clone
    for ConcurrentMarkingTrace<VM, P, KIND>
{
    fn clone(&self) -> Self {
        Self { plan: self.plan }
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind> Trace
    for ConcurrentMarkingTrace<VM, P, KIND>
{
    type VM = VM;

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

    fn may_move_objects() -> bool {
        // Concurrent marking never moves objects.
        false
    }
}

pub(crate) struct ConcurrentMarkingRootsWorkFactory<
    VM: VMBinding,
    P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>,
    const KIND: TraceKind,
> {
    pub(crate) mmtk: &'static MMTK<VM>,
    phantom_data: PhantomData<P>,
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind> Clone
    for ConcurrentMarkingRootsWorkFactory<VM, P, KIND>
{
    fn clone(&self) -> Self {
        Self {
            mmtk: self.mmtk,
            phantom_data: PhantomData,
        }
    }
}

unsafe impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    Send for ConcurrentMarkingRootsWorkFactory<VM, P, KIND>
{
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    ConcurrentMarkingRootsWorkFactory<VM, P, KIND>
{
    pub(crate) fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self {
            mmtk,
            phantom_data: PhantomData,
        }
    }

    fn is_final_mark(&mut self) -> bool {
        let pause = self
            .mmtk
            .get_plan()
            .concurrent()
            .unwrap()
            .current_pause()
            .unwrap();
        debug_assert!(
            pause == Pause::InitialMark || pause == Pause::FinalMark,
            "pause is neither InitialMark nor FinalMark.  pause: {pause:?}"
        );
        pause == Pause::FinalMark
    }

    fn create_and_schedule_root_nodes_work(&mut self, nodes: Vec<ObjectReference>) {
        let mmtk = self.mmtk;
        let work_packet = ConcurrentTraceObjects::<VM, P, KIND>::new(nodes, false);
        mmtk.scheduler.work_buckets[WorkBucketStage::Concurrent].add_no_notify(work_packet);
    }
}

impl<VM: VMBinding, P: ConcurrentPlan<VM = VM> + PlanTraceObject<VM>, const KIND: TraceKind>
    RootsWorkFactory<VM::VMSlot> for ConcurrentMarkingRootsWorkFactory<VM, P, KIND>
{
    fn create_process_roots_work(&mut self, slots: Vec<VM::VMSlot>) {
        probe!(mmtk, roots, RootsKind::NORMAL, slots.len());

        if self.is_final_mark() {
            return;
        }

        // We don't divide the `slots` vector into smaller chunks here.  We assume the VM binding
        // respects the constant `EDGES_WORK_BUFFER_SIZE` and provides lists of slots in reasonable
        // lengths.  Even if a single `ConcurrentTraceObjects` work packet is too large, it can
        // still break up the list during tracing using the constant `CONCURRENT_TRACE_OVERFLOW`.
        let nodes = slots
            .iter()
            .flat_map(|slot| slot.load())
            .collect::<Vec<_>>();

        // Note: During concurrent marking, mutators can overwrite the root slots and make the roots unstable.
        // Therefore, instead of recording the root slots, we record the loaded root nodes.
        #[cfg(feature = "sanity")]
        self.mmtk
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_nodes(nodes.clone());

        self.create_and_schedule_root_nodes_work(nodes);
    }

    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        probe!(mmtk, roots, RootsKind::PINNING, nodes.len());

        if self.is_final_mark() {
            return;
        }

        #[cfg(feature = "sanity")]
        self.mmtk
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_nodes(nodes.clone());

        self.create_and_schedule_root_nodes_work(nodes);
    }

    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        probe!(mmtk, roots, RootsKind::TPINNING, nodes.len());

        if self.is_final_mark() {
            return;
        }

        #[cfg(feature = "sanity")]
        self.mmtk
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_nodes(nodes.clone());

        self.create_and_schedule_root_nodes_work(nodes);
    }
}
