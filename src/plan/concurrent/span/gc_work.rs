use crate::plan::concurrent::barrier::HYBRID_BARRIER_WORK_FLUSHES;
use crate::plan::concurrent::concurrent_marking_work::ConcurrentMarkingRootsWorkFactory;
use crate::plan::concurrent::span::global::ConcurrentSpan;
use crate::plan::tracing::PlanTrace;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::scheduler::{GCWork, GCWorker};
use crate::vm::{Collection, VMBinding};
use crate::MMTK;
use std::sync::atomic::Ordering;

/// The `GCWorkContext` for stop-the-world `Pause::Full` collections in `ConcurrentSpan`.
pub(super) struct ConcurrentSpanSTWGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);

impl<VM: VMBinding> crate::scheduler::GCWorkContext for ConcurrentSpanSTWGCWorkContext<VM> {
    type VM = VM;
    type PlanType = ConcurrentSpan<VM>;
    type DefaultTrace = PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>;
    type PinningTrace = PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>;
}

/// The `GCWorkContext` for concurrent marking.  Roots are loaded when they are reported, and
/// traced in the `Concurrent` bucket.
pub(super) struct ConcurrentSpanGCWorkContext<VM: VMBinding>(std::marker::PhantomData<VM>);

impl<VM: VMBinding> crate::scheduler::GCWorkContext for ConcurrentSpanGCWorkContext<VM> {
    type VM = VM;
    type PlanType = ConcurrentSpan<VM>;
    type DefaultTrace = PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>;
    type PinningTrace = PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>;

    fn make_roots_work_factory(
        mmtk: &'static MMTK<Self::VM>,
    ) -> impl crate::vm::RootsWorkFactory<<Self::VM as VMBinding>::VMSlot> {
        ConcurrentMarkingRootsWorkFactory::<Self::VM, Self::PlanType, DEFAULT_TRACE>::new(mmtk)
    }
}

/// One round of Go's ragged barrier (`gcMarkDone`, `runtime/mgc.go:997-1060`): flush the barrier
/// buffer of every mutator, without stopping all mutators at once, and record whether any
/// buffer held work.  If none did, marking is done, and the plan requests `FinalMark`.
pub(super) struct RaggedFlush;

impl<VM: VMBinding> GCWork<VM> for RaggedFlush {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let plan = mmtk
            .get_plan()
            .downcast_ref::<ConcurrentSpan<VM>>()
            .unwrap();
        let before = HYBRID_BARRIER_WORK_FLUSHES.load(Ordering::SeqCst);
        VM::VMCollection::flush_mutators_concurrently(worker.tls);
        let found_work = HYBRID_BARRIER_WORK_FLUSHES.load(Ordering::SeqCst) != before;
        plan.on_ragged_flush_done(found_work);
    }
}

/// Scan the contents of every finalizable object, without marking the object itself (Go's
/// `gcScanFinalizer`, `runtime/mgcmark.go:393-493`).  If such an object turns out to be
/// unreachable at the end of marking, `Finalization` resurrects it in the `FinalMark` pause, and
/// everything it references is already marked, so the pause does not have to trace from it.
pub(super) struct ScanFinalizableContents;

impl<VM: VMBinding> GCWork<VM> for ScanFinalizableContents {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        use crate::plan::concurrent::concurrent_marking_work::ConcurrentTraceObjects;
        use crate::scheduler::WorkBucketStage;
        let objects = mmtk
            .finalizable_processor
            .lock()
            .unwrap()
            .candidate_references();
        for chunk in objects.chunks(crate::scheduler::EDGES_WORK_BUFFER_SIZE) {
            // "Already marked" makes the packet scan the objects without marking them.
            worker.add_work(
                WorkBucketStage::Concurrent,
                ConcurrentTraceObjects::<VM, ConcurrentSpan<VM>, DEFAULT_TRACE>::new(
                    chunk.to_vec(),
                    true,
                ),
            );
        }
    }
}

/// The first packet of a `FinalMark` pause (Go's mark termination, `runtime/mgc.go:1066-1156`).
/// The mutators' barrier buffers have just been flushed.  If that produced marking work, marking
/// restarts concurrently.  Otherwise it schedules the rest of the pause.
pub(super) struct FinalMarkCheck;

impl<VM: VMBinding> GCWork<VM> for FinalMarkCheck {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let plan = mmtk
            .get_plan()
            .downcast_ref::<ConcurrentSpan<VM>>()
            .unwrap();
        plan.final_mark_check(mmtk);
    }
}
