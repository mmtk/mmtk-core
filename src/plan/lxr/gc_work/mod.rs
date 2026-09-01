use super::global::LXR;
use crate::plan::tracing::UnsupportedTrace;
use crate::plan::VectorObjectQueue;
use crate::scheduler::gc_work::RootKind;
use crate::scheduler::{GCWorker, WorkBucketStage};
use crate::util::ObjectReference;
use crate::vm::{RootsWorkFactory, VMBinding};
use crate::{Plan, MMTK};
use std::marker::PhantomData;

pub mod mature_evac;
pub mod mature_sweeping;
pub mod nursery_sweeping;
pub mod prepare;
pub mod rc;
pub mod tracing;

use rc::{CollectNodeRoots, CollectRoots};

/// Common base fields shared by LXR's custom root/closure work packets.
///
/// This used to be `crate::scheduler::gc_work::ProcessEdgesBase`. After upstream replaced
/// `ProcessEdgesWork` with the stateless `Trace` API, LXR keeps its own work-packet based
/// closures, so this helper lives locally in the LXR plan.
pub struct ProcessEdgesBase<VM: VMBinding> {
    pub slots: Vec<VM::VMSlot>,
    pub nodes: VectorObjectQueue,
    mmtk: &'static MMTK<VM>,
    // Use raw pointer for fast pointer dereferencing, instead of using `Option<&'static mut GCWorker<VM>>`.
    // Because a copying gc will dereference this pointer at least once for every object copy.
    worker: *mut GCWorker<VM>,
    pub roots: bool,
    pub root_kind: Option<RootKind>,
    pub bucket: WorkBucketStage,
}

unsafe impl<VM: VMBinding> Send for ProcessEdgesBase<VM> {}

impl<VM: VMBinding> ProcessEdgesBase<VM> {
    pub fn new(
        slots: Vec<VM::VMSlot>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        #[cfg(feature = "extreme_assertions")]
        if crate::util::slot_logger::should_check_duplicate_slots(mmtk.get_plan()) {
            for slot in &slots {
                // log slot, panic if already logged
                mmtk.slot_logger.log_slot(*slot);
            }
        }
        Self {
            slots,
            nodes: VectorObjectQueue::new(),
            mmtk,
            worker: std::ptr::null_mut(),
            roots,
            root_kind: if roots { Some(RootKind::Strong) } else { None },
            bucket,
        }
    }

    pub fn set_worker(&mut self, worker: &mut GCWorker<VM>) {
        self.worker = worker;
    }

    pub fn worker(&self) -> &'static mut GCWorker<VM> {
        unsafe { &mut *self.worker }
    }

    pub fn mmtk(&self) -> &'static MMTK<VM> {
        self.mmtk
    }

    pub fn plan(&self) -> &'static dyn Plan<VM = VM> {
        self.mmtk.get_plan()
    }
}

/// The [`crate::scheduler::GCWorkContext`] for LXR.
///
/// LXR does not use the generic `Trace`-based closures. Instead it schedules its own custom work
/// packets. The `DefaultTrace`/`PinningTrace` members are therefore set to [`UnsupportedTrace`],
/// and root scanning is routed through [`LXRRootsWorkFactory`].
pub struct LXRGCWorkContext<VM: VMBinding>(PhantomData<VM>);

impl<VM: VMBinding> crate::scheduler::GCWorkContext for LXRGCWorkContext<VM> {
    type VM = VM;
    type PlanType = LXR<VM>;
    type DefaultTrace = UnsupportedTrace<VM>;
    type PinningTrace = UnsupportedTrace<VM>;

    fn make_roots_work_factory(
        mmtk: &'static MMTK<VM>,
    ) -> impl RootsWorkFactory<<VM as VMBinding>::VMSlot> {
        LXRRootsWorkFactory::new(mmtk)
    }
}

/// The [`RootsWorkFactory`] used by LXR. Roots are processed by reference-counting them through
/// [`CollectRoots`] (which spawns `ProcessIncs`).
pub struct LXRRootsWorkFactory<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
}

impl<VM: VMBinding> Clone for LXRRootsWorkFactory<VM> {
    fn clone(&self) -> Self {
        Self { mmtk: self.mmtk }
    }
}

impl<VM: VMBinding> LXRRootsWorkFactory<VM> {
    fn new(mmtk: &'static MMTK<VM>) -> Self {
        Self { mmtk }
    }
}

impl<VM: VMBinding> RootsWorkFactory<VM::VMSlot> for LXRRootsWorkFactory<VM> {
    fn create_process_roots_work_with_root_kind(&mut self, slots: Vec<VM::VMSlot>, kind: RootKind) {
        // Slot roots may evacuate, so they run in `RCProcessIncs`.
        let stage = WorkBucketStage::RCProcessIncs;
        let mut w = CollectRoots::new(slots, true, self.mmtk, stage);
        w.root_kind = Some(kind);
        crate::memory_manager::add_work_packet(self.mmtk, stage, w);
    }

    fn create_process_pinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        self.create_node_roots_work(nodes);
    }

    fn create_process_tpinning_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        self.create_node_roots_work(nodes);
    }
}

impl<VM: VMBinding> LXRRootsWorkFactory<VM> {
    /// Reference-counts and, if needed, traces a set of roots reported as objects rather
    /// than slots.
    ///
    /// This always schedules into `RCProcessRootNodes`, regardless of pause: that stage is
    /// what reference-counts (and, for a nursery object, promotes) these roots, which is
    /// what makes them ineligible for nursery eviction. `RCProcessRootNodes` is guaranteed
    /// to fully drain before `RCProcessIncs` opens, so the counting has to happen there even
    /// when there's nothing to trace yet -- `RCProcessIncs` processes ordinary slots to the
    /// very same objects, and its nursery-eviction decision only skips an object once its rc
    /// count is already nonzero. Scheduling this work into a later stage instead (e.g.
    /// `PinningRootsTrace`, conditional on pause) would leave that window open for every
    /// pause but `RefCount`.
    ///
    /// The *tracing* of these roots, in contrast, is deferred by
    /// [`rc::ProcessIncs::process_and_schedule_root_nodes`] to whichever stage suits the
    /// current pause -- `PinningRootsTrace` for a pause with a `Closure` phase, since nodes,
    /// unlike slots, are never re-scanned and so must be traced directly before `Closure`
    /// opens; nothing for `RefCount`, which has no `Closure` phase at all. Only the counting
    /// has to be this early.
    fn create_node_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        if nodes.is_empty() {
            return;
        }
        crate::memory_manager::add_work_packet(
            self.mmtk,
            WorkBucketStage::RCProcessRootNodes,
            CollectNodeRoots::<VM>::new(nodes),
        );
    }
}
