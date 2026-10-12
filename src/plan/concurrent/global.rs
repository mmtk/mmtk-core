use crate::plan::concurrent::Pause;
use crate::plan::Plan;
use crate::scheduler::GCWorker;
use crate::util::ObjectReference;

/// Trait for a concurrent plan.
pub trait ConcurrentPlan: Plan {
    /// Return `true`` if concurrent work (such as concurrent marking) is in progress.
    fn concurrent_work_in_progress(&self) -> bool;
    /// Return the current pause kind.  `None` if not in a pause.
    fn current_pause(&self) -> Option<Pause>;
    /// Called when concurrent work is interrupted.
    fn on_concurrent_work_interrupted(&self);
    /// Called when the concurrent phase's work packets have drained: every GC worker has parked,
    /// so nothing is in progress, and no pause has been requested.
    ///
    /// Return `true` if the plan added more concurrent work (with `add_no_notify`, because the
    /// caller holds the lock that the usual notification needs) or changed its state so that a
    /// pause is now required.  The last parked worker then looks for work again, instead of
    /// parking.
    fn on_concurrent_work_drained(&self) -> bool;
    /// Return `true` if a mutator that needs a collection now (for example because the heap is
    /// full) should wait for the concurrent work in progress to finish, instead of requesting a
    /// pause that interrupts it.  The mutator then blocks in
    /// [`crate::vm::Collection::block_for_gc`] until the next pause ends.  This is an
    /// *allocation stall*.
    fn stall_instead_of_pause(&self) -> bool {
        false
    }
    /// Called when a mutator is about to stall (see [`ConcurrentPlan::stall_instead_of_pause`]).
    fn on_allocation_stall(&self) {}
    /// Called when a stalled mutator resumes, with the time it was stalled.
    fn on_allocation_stall_finished(&self, _duration: std::time::Duration) {}
    /// Return `true` if the plan wants [`ConcurrentPlan::add_scan_work`] to be called.
    fn counts_scan_work(&self) -> bool {
        false
    }
    /// Report scan work done by concurrent marking, in bytes: heap objects scanned, and roots
    /// found on stacks and elsewhere (Go's `heapScanWork`, `stackScanWork` and
    /// `globalsScanWork`).  Only called if [`ConcurrentPlan::counts_scan_work`] returns `true`.
    fn add_scan_work(&self, _heap: usize, _stack: usize, _globals: usize) {}
    /// See [`crate::memory_manager::shade_objects`].
    fn shade_objects(&self, _objects: &[crate::util::ObjectReference]) {}
    /// See [`crate::memory_manager::soft_references_are_strong`].
    fn soft_references_are_strong(&self) -> bool {
        false
    }
    /// Let the plan trace a packet of concurrent marking work with its own tracing loop, instead
    /// of the generic one in [`super::concurrent_marking_work::ConcurrentTraceObjects`]: the root
    /// nodes, barrier buffers and rescans that `ConcurrentTraceObjects` and `ProcessModBufSATB`
    /// packets carry.  `already_marked` says whether `objects` are marked (and only need
    /// scanning) or still have to be traced.
    ///
    /// Return `None` if the plan has traced the objects, or give them back to use the generic
    /// loop (the default).
    fn trace_objects_concurrently(
        &self,
        _worker: &mut GCWorker<Self::VM>,
        objects: Vec<ObjectReference>,
        _already_marked: bool,
    ) -> Option<Vec<ObjectReference>> {
        Some(objects)
    }
}
