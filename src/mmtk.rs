///! MMTk instance.
use crate::plan::Plan;
use crate::policy::space::SFTMap;
use crate::scheduler::GCWorkScheduler;
use crate::util::finalizable_processor::FinalizableProcessor;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::map::Map;
use crate::util::opaque_pointer::*;
use crate::util::options::{Options, UnsafeOptionsWrapper};
use crate::util::reference_processor::ReferenceProcessors;
#[cfg(feature = "sanity")]
use crate::util::sanity::sanity_checker::SanityChecker;
use crate::vm::ReferenceGlue;
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

    /// A global VMMap that manages the mapping of spaces to virtual memory ranges.
    pub static ref VM_MAP: VMMap = VMMap::new();

    /// A global Mmapper for mmaping and protection of virtual memory.
    pub static ref MMAPPER: Mmapper = Mmapper::new();
}

use crate::util::rust_util::InitializeOnce;

// A global space function table that allows efficient dispatch space specific code for addresses in our heap.
pub static SFT_MAP: InitializeOnce<SFTMap<'static>> = InitializeOnce::new();

/// An MMTk instance. MMTk allows multiple instances to run independently, and each instance gives users a separate heap.
/// *Note that multi-instances is not fully supported yet*
pub struct MMTK<VM: VMBinding> {
    pub(crate) plan: Box<dyn Plan<VM = VM>>,
    pub(crate) reference_processors: ReferenceProcessors,
    pub(crate) finalizable_processor:
        Mutex<FinalizableProcessor<<VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType>>,
    pub options: Arc<UnsafeOptionsWrapper>,
    pub(crate) scheduler: Arc<GCWorkScheduler<VM>>,
    #[cfg(feature = "sanity")]
    pub(crate) sanity_checker: Mutex<SanityChecker>,
    inside_harness: AtomicBool,
}

impl<VM: VMBinding> MMTK<VM> {
    pub fn new() -> Self {
        // Initialize SFT first in case we need to use this in the constructor.
        // The first call will initialize SFT map. Other calls will be blocked until SFT map is initialized.
        SFT_MAP.initialize_once(&SFTMap::new);

        let options = Arc::new(UnsafeOptionsWrapper::new(Options::default()));

        let num_workers = if cfg!(feature = "single_worker") {
            1
        } else {
            *options.threads
        };

        let scheduler = GCWorkScheduler::new(num_workers);
        let plan = crate::plan::create_plan(
            *options.plan,
            &VM_MAP,
            &MMAPPER,
            options.clone(),
            scheduler.clone(),
        );
        MMTK {
            plan,
            reference_processors: ReferenceProcessors::new(),
            finalizable_processor: Mutex::new(FinalizableProcessor::<
                <VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType,
            >::new()),
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

    pub fn get_plan(&self) -> &dyn Plan<VM = VM> {
        self.plan.as_ref()
    }

    #[inline(always)]
    pub fn get_options(&self) -> &Options {
        &self.options
    }
}

impl<VM: VMBinding> Default for MMTK<VM> {
    fn default() -> Self {
        Self::new()
    }
}
