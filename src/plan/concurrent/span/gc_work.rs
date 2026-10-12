use crate::plan::concurrent::barrier::BARRIER_WORK_FLUSHES;
use crate::plan::concurrent::concurrent_marking_work::ConcurrentMarkingRootsWorkFactory;
use crate::plan::concurrent::global::ConcurrentPlan;
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
        if plan.current_pause().is_some() {
            // A pause interrupted marking before this round ran.  The mutators were flushed when
            // they stopped, and a VM may not be able to flush them concurrently in a pause.
            return;
        }
        let before = BARRIER_WORK_FLUSHES.load(Ordering::SeqCst);
        VM::VMCollection::flush_mutators_concurrently(worker.tls);
        let found_work = BARRIER_WORK_FLUSHES.load(Ordering::SeqCst) != before;
        plan.on_ragged_flush_done(found_work);
    }
}

/// Opens the `Concurrent` bucket after `InitialMark` when the pause left no concurrent work, for
/// example when stacks are scanned in the pause.  The end of marking is detected when the last
/// GC worker parks with `Concurrent` drained, and without a packet no worker would wake up to
/// park again.
pub(super) struct WakeConcurrentWorkers;

impl<VM: VMBinding> GCWork<VM> for WakeConcurrentWorkers {
    fn do_work(&mut self, _worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {}
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
