use libc::c_void;
use ::util::ObjectReference;
use super::{MutatorContext, CollectorContext, ParallelCollector, TraceLocal};
use plan::phase::{Phase, Schedule, ScheduledPhase};
use std::sync::atomic::{self, AtomicUsize, AtomicBool, Ordering};
use ::util::OpaquePointer;
use ::policy::space::Space;
use ::util::heap::PageResource;
use ::util::options::OPTION_MAP;
use ::vm::{Collection, VMCollection, ActivePlan, VMActivePlan};
use super::controller_collector_context::ControllerCollectorContext;
use util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use util::constants::LOG_BYTES_IN_MBYTE;
use util::heap::VMRequest;
use policy::immortalspace::ImmortalSpace;
#[cfg(feature = "jikesrvm")]
use vm::jikesrvm::heap_layout_constants::BOOT_IMAGE_END;
#[cfg(feature = "jikesrvm")]
use vm::jikesrvm::heap_layout_constants::BOOT_IMAGE_DATA_START;
use util::Address;
use util::heap::pageresource::cumulative_committed_pages;
use util::statistics::stats::{STATS, get_gathering_stats, new_counter};
use util::statistics::counter::{Counter, LongCounter};
use util::statistics::counter::MonotoneNanoTime;
use util::heap::layout::heap_layout::VMMap;
use util::heap::layout::heap_layout::Mmapper;
use util::heap::layout::Mmapper as IMmapper;

pub static EMERGENCY_COLLECTION: AtomicBool = AtomicBool::new(false);
pub static USER_TRIGGERED_COLLECTION: AtomicBool = AtomicBool::new(false);

lazy_static! {
    pub static ref CONTROL_COLLECTOR_CONTEXT: ControllerCollectorContext = ControllerCollectorContext::new();
}

// FIXME: Move somewhere more appropriate
#[cfg(feature = "jikesrvm")]
pub fn create_vm_space(vm_map: &'static VMMap, mmapper: &'static Mmapper) -> ImmortalSpace {
    let boot_segment_bytes = BOOT_IMAGE_END - BOOT_IMAGE_DATA_START;
    debug_assert!(boot_segment_bytes > 0);

    let boot_segment_mb = unsafe{Address::from_usize(boot_segment_bytes)}
        .align_up(BYTES_IN_CHUNK).as_usize() >> LOG_BYTES_IN_MBYTE;

    ImmortalSpace::new("boot", false, VMRequest::fixed_size(boot_segment_mb), vm_map, mmapper)
}

#[cfg(feature = "openjdk")]
pub fn create_vm_space(vm_map: &'static VMMap, mmapper: &'static Mmapper) -> ImmortalSpace {
    // FIXME: Does OpenJDK care?
    ImmortalSpace::new("boot", false, VMRequest::fixed_size(0), vm_map, mmapper)
}

pub trait Plan: Sized {
    type MutatorT: MutatorContext;
    type TraceLocalT: TraceLocal;
    type CollectorT: ParallelCollector;

    fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper) -> Self;
    fn mmapper(&self) -> &'static Mmapper;
    // unsafe because this can only be called once by the init thread
    unsafe fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap);
    fn bind_mutator(&'static self, tls: OpaquePointer) -> *mut c_void;
    fn will_never_move(&self, object: ObjectReference) -> bool;
    // unsafe because only the primary collector thread can call this
    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase);

    fn is_initialized() -> bool {
        INITIALIZED.load(Ordering::SeqCst)
    }

    fn poll<PR: PageResource>(&self, space_full: bool, space: &'static PR::Space) -> bool {
        if self.collection_required::<PR>(space_full, space) {
            // FIXME
            /*if space == META_DATA_SPACE {
                /* In general we must not trigger a GC on metadata allocation since
                 * this is not, in general, in a GC safe point.  Instead we initiate
                 * an asynchronous GC, which will occur at the next safe point.
                 */
                self.log_poll(space, "Asynchronous collection requested");
                self.common().control_collector_context.request();
                return false;
            }*/
            self.log_poll::<PR>(space, "Triggering collection");
            CONTROL_COLLECTOR_CONTEXT.request();
            return true;
        }

        // FIXME
        /*if self.concurrent_collection_required() {
            // FIXME
            /*if space == self.common().meta_data_space {
                self.log_poll(space, "Triggering async concurrent collection");
                Self::trigger_internal_collection_request();
                return false;
            } else {*/
            self.log_poll(space, "Triggering concurrent collection");
            Self::trigger_internal_collection_request();
            return true;
        }*/

        return false;
    }

    fn log_poll<PR: PageResource>(&self, space: &'static PR::Space, message: &'static str) {
        if OPTION_MAP.verbose >= 5 {
            println!("  [POLL] {}: {}", space.get_name(), message);
        }
    }

    /**
     * This method controls the triggering of a GC. It is called periodically
     * during allocation. Returns <code>true</code> to trigger a collection.
     *
     * @param spaceFull Space request failed, must recover pages within 'space'.
     * @param space TODO
     * @return <code>true</code> if a collection is requested by the plan.
     */
    fn collection_required<PR: PageResource>(&self, space_full: bool, space: &'static PR::Space) -> bool where Self: Sized {
        let stress_force_gc = self.stress_test_gc_required();
        trace!("self.get_pages_reserved()={}, self.get_total_pages()={}",
               self.get_pages_reserved(), self.get_total_pages());
        let heap_full = self.get_pages_reserved() > self.get_total_pages();

        space_full || stress_force_gc || heap_full
    }

    fn get_pages_reserved(&self) -> usize {
        self.get_pages_used() + self.get_collection_reserve()
    }

    fn get_total_pages(&self) -> usize;

    fn get_pages_avail(&self) -> usize {
        self.get_total_pages() - self.get_pages_reserved()
    }

    fn get_collection_reserve(&self) -> usize {
        0
    }

    fn get_pages_used(&self) -> usize;

    fn is_emergency_collection() -> bool {
        EMERGENCY_COLLECTION.load(Ordering::Relaxed)
    }

    fn get_free_pages(&self) -> usize { self.get_total_pages() - self.get_pages_used() }

    #[inline]
    fn stress_test_gc_required(&self) -> bool {
        let pages = cumulative_committed_pages();
        trace!("pages={}", pages);

        if INITIALIZED.load(Ordering::Relaxed)
            && (pages ^ LAST_STRESS_PAGES.load(Ordering::Relaxed)
            > OPTION_MAP.stress_factor) {

            LAST_STRESS_PAGES.store(pages, Ordering::Relaxed);
            trace!("Doing stress GC");
            true
        } else {
            false
        }
    }

    fn is_internal_triggered_collection() -> bool {
        // FIXME
        false
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        true
    }

    fn force_full_heap_collection(&self) {

    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool;

    fn is_bad_ref(&self, object: ObjectReference) -> bool;

    fn handle_user_collection_request(tls: OpaquePointer) {
        if !OPTION_MAP.ignore_system_g_c {
            USER_TRIGGERED_COLLECTION.store(true, Ordering::Relaxed);
            CONTROL_COLLECTOR_CONTEXT.request();
            VMCollection::block_for_gc(tls);
        }
    }

    fn is_user_triggered_collection() -> bool {
        return USER_TRIGGERED_COLLECTION.load(Ordering::Relaxed);
    }

    fn reset_collection_trigger() {
        USER_TRIGGERED_COLLECTION.store(false, Ordering::Relaxed)
    }

    fn is_mapped_object(&self, object: ObjectReference) -> bool {
        if object.is_null() {
            return false;
        }
        if !self.is_valid_ref(object) {
            return false;
        }
        if !self.mmapper().object_is_mapped(object) {
            return false;
        }
        true
    }

    fn is_mapped_address(&self, address: Address) -> bool;

    fn modify_check(&self, object: ObjectReference) {
        if gc_in_progress_proper() {
            if self.is_movable(object) {
                panic!("GC modifying a potentially moving object via Java (i.e. not magic) obj= {}", object);
            }
        }
    }

    fn is_movable(&self, object: ObjectReference) -> bool;
}

#[derive(PartialEq)]
pub enum GcStatus {
    NotInGC,
    GcPrepare,
    GcProper,
}

pub static INITIALIZED: AtomicBool = AtomicBool::new(false);
// FIXME should probably not use static mut
static mut GC_STATUS: GcStatus = GcStatus::NotInGC;
static LAST_STRESS_PAGES: AtomicUsize = AtomicUsize::new(0);
pub static STACKS_PREPARED: AtomicBool = AtomicBool::new(false);


lazy_static! {
    pub static ref PREPARE_STACKS: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Mutator, Phase::PrepareStacks),
        ScheduledPhase::new(Schedule::Global, Phase::PrepareStacks)
    ], 0, None);

    pub static ref SANITY_BUILD_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Global, Phase::SanityPrepare),
        ScheduledPhase::new(Schedule::Collector, Phase::SanityPrepare),
        ScheduledPhase::new(Schedule::Complex, PREPARE_STACKS.clone()),
        ScheduledPhase::new(Schedule::Collector, Phase::SanityRoots),
        ScheduledPhase::new(Schedule::Global, Phase::SanityRoots),
        ScheduledPhase::new(Schedule::Collector, Phase::SanityCopyRoots),
        ScheduledPhase::new(Schedule::Global, Phase::SanityBuildTable)
    ], 0, None);

    pub static ref SANITY_CHECK_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Global, Phase::SanityCheckTable),
        ScheduledPhase::new(Schedule::Collector, Phase::SanityRelease),
        ScheduledPhase::new(Schedule::Global, Phase::SanityRelease)
    ], 0, None);

    pub static ref INIT_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Global, Phase::SetCollectionKind),
        ScheduledPhase::new(Schedule::Global, Phase::Initiate),
        ScheduledPhase::new(Schedule::Placeholder, Phase::PreSanityPlaceholder)
    ], 0, Some(new_counter(LongCounter::<MonotoneNanoTime>::new("init".to_string(), false, true))));

    pub static ref ROOT_CLOSURE_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Mutator, Phase::Prepare),
        ScheduledPhase::new(Schedule::Global, Phase::Prepare),
        ScheduledPhase::new(Schedule::Collector, Phase::Prepare),
        ScheduledPhase::new(Schedule::Complex, PREPARE_STACKS.clone()),
        ScheduledPhase::new(Schedule::Collector, Phase::StackRoots),
        ScheduledPhase::new(Schedule::Global, Phase::StackRoots),
        ScheduledPhase::new(Schedule::Collector, Phase::Roots),
        ScheduledPhase::new(Schedule::Global, Phase::Roots),
        ScheduledPhase::new(Schedule::Global, Phase::Closure),
        ScheduledPhase::new(Schedule::Collector, Phase::Closure)
    ], 0, None);

    pub static ref REF_TYPE_CLOSURE_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Collector, Phase::SoftRefs),
        ScheduledPhase::new(Schedule::Global, Phase::Closure),
        ScheduledPhase::new(Schedule::Collector, Phase::Closure),
        ScheduledPhase::new(Schedule::Collector, Phase::WeakRefs),
        ScheduledPhase::new(Schedule::Collector, Phase::Finalizable),
        ScheduledPhase::new(Schedule::Global, Phase::Closure),
        ScheduledPhase::new(Schedule::Collector, Phase::Closure),
        ScheduledPhase::new(Schedule::Placeholder, Phase::WeakTrackRefs),
        ScheduledPhase::new(Schedule::Collector, Phase::PhantomRefs)
    ], 0, None);

    pub static ref FORWARD_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Placeholder, Phase::Forward),
        ScheduledPhase::new(Schedule::Collector, Phase::ForwardRefs),
        ScheduledPhase::new(Schedule::Collector, Phase::ForwardFinalizable)
    ], 0, None);

    pub static ref COMPLETE_CLOSURE_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Mutator, Phase::Release),
        ScheduledPhase::new(Schedule::Collector, Phase::Release),
        ScheduledPhase::new(Schedule::Global, Phase::Release)
    ], 0, None);

    pub static ref FINISH_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Placeholder, Phase::PostSanityPlaceholder),
        ScheduledPhase::new(Schedule::Collector, Phase::Complete),
        ScheduledPhase::new(Schedule::Global, Phase::Complete)
    ], 0, Some(new_counter(LongCounter::<MonotoneNanoTime>::new("finish".to_string(), false, true))));

    pub static ref COLLECTION: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Complex, INIT_PHASE.clone()),
        ScheduledPhase::new(Schedule::Complex, ROOT_CLOSURE_PHASE.clone()),
        ScheduledPhase::new(Schedule::Complex, REF_TYPE_CLOSURE_PHASE.clone()),
        ScheduledPhase::new(Schedule::Complex, FORWARD_PHASE.clone()),
        ScheduledPhase::new(Schedule::Complex, COMPLETE_CLOSURE_PHASE.clone()),
        ScheduledPhase::new(Schedule::Complex, FINISH_PHASE.clone())
    ], 0, None);

    pub static ref PRE_SANITY_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Global, Phase::SanitySetPreGC),
        ScheduledPhase::new(Schedule::Complex, SANITY_BUILD_PHASE.clone()),
        ScheduledPhase::new(Schedule::Complex, SANITY_CHECK_PHASE.clone())
    ], 0, None);

    pub static ref POST_SANITY_PHASE: Phase = Phase::Complex(vec![
        ScheduledPhase::new(Schedule::Global, Phase::SanitySetPostGC),
        ScheduledPhase::new(Schedule::Complex, SANITY_BUILD_PHASE.clone()),
        ScheduledPhase::new(Schedule::Complex, SANITY_CHECK_PHASE.clone())
    ], 0, None);
}

#[repr(i32)]
#[derive(Clone, Copy, Debug)]
pub enum Allocator {
    Default = 0,
    NonReference = 1,
    NonMoving = 2,
    Immortal = 3,
    Los = 4,
    PrimitiveLos = 5,
    GcSpy = 6,
    Code = 7,
    LargeCode = 8,
    Allocators = 9,
    DefaultSite = -1,
}

pub fn set_gc_status(s: GcStatus) {
    if unsafe { GC_STATUS == GcStatus::NotInGC } {
        STACKS_PREPARED.store(false, Ordering::SeqCst);
        // FIXME stats
        STATS.lock().unwrap().start_gc();

    }
    unsafe { GC_STATUS = s };
    if unsafe { GC_STATUS == GcStatus::NotInGC } {
        // FIXME stats
        if get_gathering_stats() {
            STATS.lock().unwrap().end_gc();
        }
    }
}

pub fn stacks_prepared() -> bool {
    STACKS_PREPARED.load(Ordering::SeqCst)
}

pub fn gc_in_progress() -> bool {
    unsafe { GC_STATUS != GcStatus::NotInGC }
}

pub fn gc_in_progress_proper() -> bool {
    unsafe { GC_STATUS == GcStatus::GcProper }
}

static INSIDE_HARNESS: AtomicBool = AtomicBool::new(false);

pub fn harness_begin(tls: OpaquePointer) {
    // FIXME Do a full heap GC if we have generational GC
    let old_ignore = OPTION_MAP.ignore_system_g_c;
    unsafe { OPTION_MAP.process("ignoreSystemGC", "false"); }
    ::plan::selected_plan::SelectedPlan::handle_user_collection_request(tls);
    if old_ignore {
        unsafe { OPTION_MAP.process("ignoreSystemGC", "true"); }
    } else {
        unsafe { OPTION_MAP.process("ignoreSystemGC", "false"); }
    }
    INSIDE_HARNESS.store(true, Ordering::SeqCst);
    STATS.lock().unwrap().start_all();
}

pub fn harness_end() {
    STATS.lock().unwrap().stop_all();
    INSIDE_HARNESS.store(false, Ordering::SeqCst);
}