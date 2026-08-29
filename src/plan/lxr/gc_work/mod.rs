use super::global::LXR;
use crate::plan::lxr::gc_work::rc::{ProcessIncs, EDGE_KIND_ROOT};
use crate::plan::tracing::UnsupportedTrace;
use crate::scheduler::gc_work::RootKind;
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
        let lxr = self.mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        let stage = lxr.root_scanning_stage();
        let mut w = ProcessIncs::<_, EDGE_KIND_ROOT>::new(slots, lxr);
        w.root_kind = Some(kind);
        crate::memory_manager::add_work_packet(self.mmtk, stage, w);
    }

    fn create_process_pinning_roots_work(&mut self, _nodes: Vec<ObjectReference>) {
        unreachable!("LXR does not support pinning roots");
    }

    fn create_process_tpinning_roots_work(&mut self, _nodes: Vec<ObjectReference>) {
        unreachable!("LXR does not support transitive pinning roots");
    }
}
