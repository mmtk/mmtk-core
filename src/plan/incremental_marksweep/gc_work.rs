use super::global::IncrementalMarkSweep;
use crate::plan::tracing::{PlanTrace, Trace};
use crate::policy::immix::TRACE_KIND_FAST;
use crate::scheduler::gc_work::RootKind;
use crate::scheduler::{GCWork, GCWorkContext, GCWorker, WorkBucketStage};
use crate::util::{scanning_helper, ObjectReference};
use crate::vm::slot::Slot;
use crate::vm::{RootsWorkFactory, VMBinding};
use crate::MMTK;
use std::marker::PhantomData;

/// Trace for every IncrementalMarkSweep pause.
pub type IncrementalMarkSweepTrace<VM> = PlanTrace<IncrementalMarkSweep<VM>, TRACE_KIND_FAST>;

/// STW full collection.
pub struct IncrementalMarkSweepSTWContext<VM: VMBinding>(PhantomData<VM>);
impl<VM: VMBinding> GCWorkContext for IncrementalMarkSweepSTWContext<VM> {
    type VM = VM;
    type PlanType = IncrementalMarkSweep<VM>;
    type DefaultTrace = IncrementalMarkSweepTrace<VM>;
    type PinningTrace = IncrementalMarkSweepTrace<VM>;
}

/// InitialMark / Increment / FinalMark.
pub struct IncrementalMarkSweepIncContext<VM: VMBinding>(PhantomData<VM>);
impl<VM: VMBinding> GCWorkContext for IncrementalMarkSweepIncContext<VM> {
    type VM = VM;
    type PlanType = IncrementalMarkSweep<VM>;
    type DefaultTrace = IncrementalMarkSweepTrace<VM>;
    type PinningTrace = IncrementalMarkSweepTrace<VM>;

    fn make_roots_work_factory(
        mmtk: &'static MMTK<VM>,
    ) -> impl RootsWorkFactory<<VM as VMBinding>::VMSlot> {
        IncrementalMarkSweepRootsFactory::<VM> { mmtk }
    }
}

pub struct IncrementalMarkSweepRootsFactory<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
}
impl<VM: VMBinding> Clone for IncrementalMarkSweepRootsFactory<VM> {
    fn clone(&self) -> Self {
        Self { mmtk: self.mmtk }
    }
}
impl<VM: VMBinding> IncrementalMarkSweepRootsFactory<VM> {
    fn add_nodes(&self, nodes: Vec<ObjectReference>) {
        if nodes.is_empty() {
            return;
        }
        #[cfg(feature = "sanity")]
        self.mmtk
            .sanity_checker
            .lock()
            .unwrap()
            .add_root_nodes(nodes.clone());
        self.mmtk.scheduler.work_buckets[WorkBucketStage::Closure].add(IncrementalMarkSweepMark::<
            VM,
        >::new(
            nodes,
            false,
            WorkBucketStage::Closure,
        ));
    }
}
impl<VM: VMBinding> RootsWorkFactory<VM::VMSlot> for IncrementalMarkSweepRootsFactory<VM> {
    fn create_process_roots_work_with_root_kind(
        &mut self,
        slots: Vec<VM::VMSlot>,
        _kind: RootKind,
    ) {
        self.add_nodes(slots.iter().filter_map(|s| s.load()).collect());
    }
    #[cfg(feature = "non_moving_root")]
    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        self.add_nodes(nodes);
    }
    #[cfg(feature = "non_moving_root")]
    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        self.add_nodes(nodes);
    }
}

/// Time-bounded depth-first marking. Checks the deadline every
/// `incremental_marksweep_trace_cost_to_check_yield` cost units, where
/// `cost = 2 * objects + pointers`. On yield, spills the unscanned (marked) objects to the
/// gray pool.
pub struct IncrementalMarkSweepMark<VM: VMBinding> {
    objects: Vec<ObjectReference>,
    already_marked: bool,
    stage: WorkBucketStage,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> IncrementalMarkSweepMark<VM> {
    /// Split threshold: offload half of the stack to a new packet above this size.
    const OVERFLOW: usize = 8192;

    pub fn new(
        objects: Vec<ObjectReference>,
        already_marked: bool,
        stage: WorkBucketStage,
    ) -> Self {
        Self {
            objects,
            already_marked,
            stage,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for IncrementalMarkSweepMark<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let plan = mmtk
            .get_plan()
            .downcast_ref::<IncrementalMarkSweep<VM>>()
            .unwrap();
        let trace = IncrementalMarkSweepTrace::<VM>::from_mmtk(mmtk);
        let tls = worker.tls;
        let check_every = *mmtk
            .get_options()
            .incremental_marksweep_trace_cost_to_check_yield;

        let mut stack: Vec<ObjectReference> = Vec::new();
        let initial = std::mem::take(&mut self.objects);
        // Once the increment ends, spill queued packets without scanning
        if plan.increment_ending() {
            if self.already_marked {
                plan.gray.push(initial);
            } else {
                plan.satb.push(initial);
            }
            return;
        }
        if self.already_marked {
            stack = initial;
        } else {
            for o in initial {
                trace.trace_object(worker, o, &mut |e: ObjectReference| stack.push(e));
            }
        }

        let mut cost = 0usize;
        while let Some(object) = stack.pop() {
            let mut pointers = 0usize;
            scanning_helper::visit_children_non_moving::<VM>(tls, object, &mut |child| {
                pointers += 1;
                trace.trace_object(worker, child, &mut |e: ObjectReference| stack.push(e))
            });
            trace.post_scan_object(object);
            cost += 2 + pointers;
            if cost > check_every {
                cost = 0;
                if plan.should_yield() {
                    plan.gray.push(stack);
                    return;
                }
            }
            if stack.len() >= Self::OVERFLOW {
                let half: Vec<ObjectReference> = stack.drain(..Self::OVERFLOW / 2).collect();
                worker.add_work(
                    self.stage,
                    IncrementalMarkSweepMark::<VM>::new(half, true, self.stage),
                );
            }
        }
    }
}

/// First packet of `Increment` / `FinalMark`: turns the pools into `IncrementalMarkSweepMark` packets.
pub struct IncrementalMarkSweepResumeMarking<VM: VMBinding> {
    stage: WorkBucketStage,
    _p: PhantomData<VM>,
}
impl<VM: VMBinding> IncrementalMarkSweepResumeMarking<VM> {
    pub fn new(stage: WorkBucketStage) -> Self {
        Self {
            stage,
            _p: PhantomData,
        }
    }
}
impl<VM: VMBinding> GCWork<VM> for IncrementalMarkSweepResumeMarking<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let plan = mmtk
            .get_plan()
            .downcast_ref::<IncrementalMarkSweep<VM>>()
            .unwrap();
        let bucket = &mmtk.scheduler.work_buckets[self.stage];
        for chunk in plan.satb.take_all() {
            bucket.add(IncrementalMarkSweepMark::<VM>::new(
                chunk, false, self.stage,
            ));
        }
        for chunk in plan.gray.take_all() {
            bucket.add(IncrementalMarkSweepMark::<VM>::new(chunk, true, self.stage));
        }
    }
}

/// Time-bounded sweeping of the blocks the last `FinalMark` left unswept.
pub struct IncrementalMarkSweepSweep<VM: VMBinding>(PhantomData<VM>);
impl<VM: VMBinding> IncrementalMarkSweepSweep<VM> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
impl<VM: VMBinding> GCWork<VM> for IncrementalMarkSweepSweep<VM> {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let plan = mmtk
            .get_plan()
            .downcast_ref::<IncrementalMarkSweep<VM>>()
            .unwrap();
        plan.ms.sweep_unswept_blocks(|| plan.should_yield());
    }
}

/// See [`IncrementalMarkSweep::on_marking_drained`].
pub struct IncrementalMarkSweepEndOfMarking<VM: VMBinding>(PhantomData<VM>);
impl<VM: VMBinding> IncrementalMarkSweepEndOfMarking<VM> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
impl<VM: VMBinding> GCWork<VM> for IncrementalMarkSweepEndOfMarking<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let plan = mmtk
            .get_plan()
            .downcast_ref::<IncrementalMarkSweep<VM>>()
            .unwrap();
        plan.on_marking_drained(worker.scheduler());
    }
}
