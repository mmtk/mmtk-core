use super::global::LXR;
use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::lxr::{MATURE_EVACUATION, NURSERY_EVACUATION};
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

use rc::CollectRoots;

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
        let stage = self.mmtk.get_plan().root_scanning_stage();
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
    /// Handle roots reported as objects rather than as slots.
    ///
    /// Bindings report these when they cannot hand out the location a reference lives in
    /// (Julia does so for conservatively scanned stacks), and ask for the referents to be
    /// pinned. LXR has no pinning support: it runs its own root path through
    /// `ProcessIncs`, and the buckets the general pinning implementation uses
    /// (`PinningRootsTrace`, `TPinningClosure`) are disabled in `LXR::schedule_collection`.
    ///
    /// Treating them as ordinary strong roots is only sound because nothing moves. That
    /// holds when both evacuation modes are compiled out, which is asserted below; a
    /// moving build must not silently drop the pinning requirement.
    fn create_node_roots_work(&mut self, nodes: Vec<ObjectReference>) {
        if NURSERY_EVACUATION || MATURE_EVACUATION {
            panic!(
                "LXR cannot honour pinning roots while evacuation is enabled; \
                 build with the `lxr_no_evac` feature"
            );
        }
        if nodes.is_empty() {
            return;
        }
        let stage = self.mmtk.get_plan().root_scanning_stage();
        crate::memory_manager::add_work_packet(
            self.mmtk,
            stage,
            CollectNodeRoots::<VM>::new(nodes),
        );
    }
}

/// Reference-counts a set of root objects reported as nodes. See
/// [`LXRRootsWorkFactory::create_node_roots_work`].
pub struct CollectNodeRoots<VM: VMBinding> {
    nodes: Vec<ObjectReference>,
    _p: PhantomData<VM>,
}

impl<VM: VMBinding> CollectNodeRoots<VM> {
    pub fn new(nodes: Vec<ObjectReference>) -> Self {
        Self {
            nodes,
            _p: PhantomData,
        }
    }
}

impl<VM: VMBinding> crate::scheduler::GCWork<VM> for CollectNodeRoots<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let lxr = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        let nodes = std::mem::take(&mut self.nodes);
        // Route through `ProcessIncs` rather than incrementing here: the zero-to-one
        // transition has to promote the object, which is what arms its field unlog bits
        // and counts the objects it refers to.
        let mut incs = rc::ProcessIncs::<VM, { rc::EDGE_KIND_ROOT }>::new(vec![], lxr);
        // `uncounted` holds the roots the plan does not reference count. They are traced
        // below alongside the counted ones, but must never reach `curr_roots`: no count was
        // raised for them, so a decrement recorded against them would be unmatched.
        let (roots, uncounted) = incs.process_root_nodes(worker, nodes);
        // Everything reachable from either set has to be marked.
        let mut to_trace = roots.clone();
        to_trace.extend_from_slice(&uncounted);
        if to_trace.is_empty() {
            return;
        }
        // While marking is running these roots also have to enter the snapshot, or the
        // cycle collector can conclude their subgraphs are unreachable. The modbuf
        // packet already takes objects, which is exactly what we have here.
        if lxr.cm_enabled() && lxr.concurrent_work_in_progress() {
            worker.add_work(
                WorkBucketStage::FinishConcurrentWork,
                tracing::ProcessModBufSATB::new(to_trace.clone()),
            );
        }
        // A tracing pause has to mark from these roots as well. `ProcessIncs` seeds the
        // stop-the-world trace from root *slots*, but roots reported as objects arrive here
        // instead and were only ever counted, never traced. With concurrent marking off,
        // nothing else marked them: the trace then reached almost nothing, and
        // `SweepDeadCycles` -- which reclaims any counted object it finds unmarked -- took the
        // live heap for cyclic garbage. Measured at the first `Full` pause of the bootstrap:
        // 189,960 objects zeroed, 0 kept as marked.
        let pause = lxr.current_pause().unwrap();
        if pause == crate::plan::concurrent::Pause::Full
            || pause == crate::plan::concurrent::Pause::FinalMark
        {
            worker.add_work(
                WorkBucketStage::Closure,
                tracing::LXRConcurrentTraceObjects::new(to_trace, mmtk),
            );
        }
        // Recorded so the matching decrements are applied in the next pause, which is
        // what makes this a root set rather than a permanent increment.
        if !roots.is_empty() {
            lxr.curr_roots.read().unwrap().push(roots);
        }
    }
}
