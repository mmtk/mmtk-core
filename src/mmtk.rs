use crate::plan::Plan;
use crate::plan::SelectedPlan;
use crate::plan::phase::PhaseManager;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::heap_layout::Mmapper;

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

    // mmtk instance
    pub static ref SINGLETON: MMTK = MMTK::new(&VM_MAP, &MMAPPER);
}

pub struct MMTK {
    pub plan: SelectedPlan,
    pub phase_manager: PhaseManager,
    pub vm_map: &'static VMMap,
    pub mmapper: &'static Mmapper,
}

impl MMTK {
    pub fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper) -> Self {
        MMTK {
            plan: SelectedPlan::new(vm_map, mmapper),
            phase_manager: PhaseManager::new(),
            vm_map,
            mmapper
        }
    }
}