///! MMTk instance.
use crate::plan::Plan;
use crate::policy::sft_map::{create_sft_map, SFTMap};
use crate::scheduler::GCWorkScheduler;

#[cfg(feature = "extreme_assertions")]
use crate::util::edge_logger::EdgeLogger;
use crate::util::finalizable_processor::FinalizableProcessor;
use crate::util::heap::layout::heap_layout::{Map, Map64};
use crate::util::heap::layout::heap_layout::{Mmapper, Mmapper32, Mmapper64};
use crate::util::heap::layout::map32::Map32;
use crate::util::opaque_pointer::*;
use crate::util::options::Options;
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
    pub static ref VM_MAP: Box<dyn Map> =  if cfg!(target_pointer_width = "32") {
        Box::new(Map32::new())
    } else {
        Box::new(Map64::new())
    };

    /// A global Mmapper for mmaping and protection of virtual memory.
    pub static ref MMAPPER: Box<dyn Mmapper> = if cfg!(target_pointer_width = "32") {
        Box::new(Mmapper32::new())
    } else {
        Box::new(Mmapper64::new())
    };
}

use crate::util::rust_util::InitializeOnce;

// A global space function table that allows efficient dispatch space specific code for addresses in our heap.
pub static SFT_MAP: InitializeOnce<Box<dyn SFTMap>> = InitializeOnce::new();

// MMTk builder. This is used to set options before actually creating an MMTk instance.
pub struct MMTKBuilder {
    /// The options for this instance.
    pub options: Options,
}

impl MMTKBuilder {
    /// Create an MMTK builder with default options
    pub fn new() -> Self {
        MMTKBuilder {
            options: Options::default(),
        }
    }

    /// Set an option.
    pub fn set_option(&mut self, name: &str, val: &str) -> bool {
        self.options.set_from_command_line(name, val)
    }

    /// Set multiple options by a string. The string should be key-value pairs separated by white spaces,
    /// such as `threads=1 stress_factor=4096`.
    pub fn set_options_bulk_by_str(&mut self, options: &str) -> bool {
        self.options.set_bulk_from_command_line(options)
    }

    /// Build an MMTk instance from the builder.
    pub fn build<VM: VMBinding>(&self) -> MMTK<VM> {
        MMTK::new(Arc::new(self.options.clone()))
    }
}

impl Default for MMTKBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// An MMTk instance. MMTk allows multiple instances to run independently, and each instance gives users a separate heap.
/// *Note that multi-instances is not fully supported yet*
pub struct MMTK<VM: VMBinding> {
    pub(crate) options: Arc<Options>,
    pub(crate) plan: Box<dyn Plan<VM = VM>>,
    pub(crate) reference_processors: ReferenceProcessors,
    pub(crate) finalizable_processor:
        Mutex<FinalizableProcessor<<VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType>>,
    pub(crate) scheduler: Arc<GCWorkScheduler<VM>>,
    #[cfg(feature = "sanity")]
    pub(crate) sanity_checker: Mutex<SanityChecker<VM::VMEdge>>,
    #[cfg(feature = "extreme_assertions")]
    pub(crate) edge_logger: EdgeLogger<VM::VMEdge>,
    inside_harness: AtomicBool,
}

impl<VM: VMBinding> MMTK<VM> {
    pub fn new(options: Arc<Options>) -> Self {
        // Initialize SFT first in case we need to use this in the constructor.
        // The first call will initialize SFT map. Other calls will be blocked until SFT map is initialized.
        SFT_MAP.initialize_once(&create_sft_map);

        let num_workers = if cfg!(feature = "single_worker") {
            1
        } else {
            *options.threads
        };

        let scheduler = GCWorkScheduler::new(num_workers, (*options.thread_affinity).clone());

        let plan = crate::plan::create_plan(
            *options.plan,
            VM_MAP.as_ref(),
            MMAPPER.as_ref(),
            options.clone(),
            scheduler.clone(),
        );

        // TODO: This probably does not work if we have multiple MMTk instances.
        VM_MAP.boot();
        // This needs to be called after we create Plan. It needs to use HeapMeta, which is gradually built when we create spaces.
        VM_MAP.finalize_static_space_map(
            plan.base().heap.get_discontig_start(),
            plan.base().heap.get_discontig_end(),
        );

        MMTK {
            options,
            plan,
            reference_processors: ReferenceProcessors::new(),
            finalizable_processor: Mutex::new(FinalizableProcessor::<
                <VM::VMReferenceGlue as ReferenceGlue<VM>>::FinalizableType,
            >::new()),
            scheduler,
            #[cfg(feature = "sanity")]
            sanity_checker: Mutex::new(SanityChecker::new()),
            inside_harness: AtomicBool::new(false),
            #[cfg(feature = "extreme_assertions")]
            edge_logger: EdgeLogger::new(),
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
