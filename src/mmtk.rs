use crate::plan::Plan;
use crate::plan::SelectedPlan;
use crate::plan::phase::PhaseManager;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::OpaquePointer;

use std::default::Default;
use crate::util::reference_processor::ReferenceProcessors;
use crate::util::options::{UnsafeOptionsWrapper, Options};
use std::sync::atomic::{Ordering, AtomicBool};

use std::sync::Arc;
use crate::vm::VMBinding;

// TODO: remove this singleton at some point to allow multiple instances of MMTK
// This helps refactoring.
lazy_static!{
    // I am not sure if we should include these mmappers as part of MMTk struct.
    // The considerations are:
    // 1. We need VMMap and Mmapper to create spaces. It is natural that the mappers are not
    //    part of MMTK, as creating MMTK requires these mappers. We could use Rc/Arc for these mappers though.
    // 2. These mmappers are possibly global across multiple MMTk instances, as they manage the
    //    entire address space.
    // TODO: We should refactor this when we know more about how multiple MMTK instances work.
    pub static ref VM_MAP: VMMap = VMMap::new();
    pub static ref MMAPPER: Mmapper = Mmapper::new();
}

pub struct MMTK<VM: VMBinding> {
    pub plan: SelectedPlan<VM>,
    pub phase_manager: PhaseManager,
    pub vm_map: &'static VMMap,
    pub mmapper: &'static Mmapper,
    pub reference_processors: ReferenceProcessors,
    pub options: Arc<UnsafeOptionsWrapper>,

    inside_harness: AtomicBool,
}

impl<VM: VMBinding> MMTK<VM> {
    pub fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper) -> Self {
        let options = Arc::new(UnsafeOptionsWrapper::new(Options::default()));
        let plan = SelectedPlan::new(vm_map, mmapper, options.clone());
        let phase_manager = PhaseManager::new(&plan.common().stats);
        MMTK {
            plan,
            phase_manager,
            vm_map,
            mmapper,
            reference_processors: ReferenceProcessors::new(),
            options,
            inside_harness: AtomicBool::new(false),
        }
    }

    pub fn harness_begin(&self, tls: OpaquePointer) {
        // FIXME Do a full heap GC if we have generational GC
        self.plan.handle_user_collection_request(tls, true);
        self.inside_harness.store(true, Ordering::SeqCst);
        self.plan.common().stats.start_all();
    }

    pub fn harness_end(&self) {
        self.plan.common().stats.stop_all();
        self.inside_harness.store(false, Ordering::SeqCst);
    }
}