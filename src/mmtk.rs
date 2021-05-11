use crate::plan::Plan;
use crate::policy::space::SFTMap;
use crate::scheduler::Scheduler;
use crate::util::finalizable_processor::FinalizableProcessor;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::map::Map;
use crate::util::opaque_pointer::*;
use crate::util::options::{Options, UnsafeOptionsWrapper};
use crate::util::reference_processor::ReferenceProcessors;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::SanityChecker;
use crate::vm::VMBinding;
use std::default::Default;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

lazy_static! {
    // I am not sure if we should include these mmappers as part of MMTk struct.
    // The considerations are:
    // 1. We need VMMap and Mmapper to create spaces. It is natural that the mappers are not
    //    part of MMTK, as creating MMTK requires these mappers. We could use Rc/Arc for these mappers though.
    // 2. These mmappers are possibly global across multiple MMTk instances, as they manage the
    //    entire address space.
    // TODO: We should refactor this when we know more about how multiple MMTK instances work.
    pub static ref VM_MAP: VMMap = VMMap::new();
    pub static ref MMAPPER: Mmapper = Mmapper::new();
    pub static ref SFT_MAP: SFTMap<'static> = SFTMap::new();
}

/// An MMTk instance. MMTk allows mutiple instances to run independently, and each instance gives users a separate heap.
/// *Note that multi-instances is not fully supported yet*
pub struct MMTK<VM: VMBinding> {
    pub plan: Box<dyn Plan<VM = VM>>,
    pub vm_map: &'static VMMap,
    pub mmapper: &'static Mmapper,
    pub sftmap: &'static SFTMap<'static>,
    pub reference_processors: ReferenceProcessors,
    pub finalizable_processor: Mutex<FinalizableProcessor>,
    pub options: Arc<UnsafeOptionsWrapper>,
    pub scheduler: Arc<Scheduler<Self>>,
    #[cfg(feature = "sanity")]
    pub sanity_checker: Mutex<SanityChecker>,
    inside_harness: AtomicBool,
}

impl<VM: VMBinding> MMTK<VM> {
    pub fn new() -> Self {
        let scheduler = Scheduler::new();
        let options = Arc::new(UnsafeOptionsWrapper::new(Options::default()));
        let plan =
            crate::plan::global::create_plan(options.plan, &VM_MAP, &MMAPPER, options.clone());
        MMTK {
            plan,
            vm_map: &VM_MAP,
            mmapper: &MMAPPER,
            sftmap: &SFT_MAP,
            reference_processors: ReferenceProcessors::new(),
            finalizable_processor: Mutex::new(FinalizableProcessor::new()),
            options,
            scheduler,
            #[cfg(feature = "sanity")]
            sanity_checker: Mutex::new(SanityChecker::new()),
            inside_harness: AtomicBool::new(false),
        }
    }

    pub fn harness_begin(&self, tls: VMMutatorThread) {
        // FIXME Do a full heap GC if we have generational GC
        self.plan.handle_user_collection_request(tls, true);
        self.inside_harness.store(true, Ordering::SeqCst);
        self.plan.base().stats.start_all();
        self.scheduler.enable_stat();
    }

    pub fn harness_end(&'static self) {
        self.plan.base().stats.stop_all(self);
        self.inside_harness.store(false, Ordering::SeqCst);
    }
}

impl<VM: VMBinding> Default for MMTK<VM> {
    fn default() -> Self {
        Self::new()
    }
}
