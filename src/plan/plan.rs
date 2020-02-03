use libc::c_void;
use ::util::ObjectReference;
use super::{MutatorContext, CollectorContext, ParallelCollector, TraceLocal};
use plan::phase::{Phase, Schedule, ScheduledPhase};
use std::sync::atomic::{self, AtomicUsize, AtomicBool, Ordering};
use ::util::OpaquePointer;
use ::policy::space::Space;
use ::util::heap::PageResource;
use ::vm::{Collection, VMCollection, ActivePlan, VMActivePlan};
use super::controller_collector_context::ControllerCollectorContext;
use util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
use util::constants::LOG_BYTES_IN_MBYTE;
use util::heap::{VMRequest, HeapMeta};
use policy::immortalspace::ImmortalSpace;
#[cfg(feature = "jikesrvm")]
use vm::jikesrvm::heap_layout_constants::BOOT_IMAGE_END;
#[cfg(feature = "jikesrvm")]
use vm::jikesrvm::heap_layout_constants::BOOT_IMAGE_DATA_START;
use util::Address;
use util::statistics::stats::{STATS, get_gathering_stats, new_counter};
use util::statistics::counter::{Counter, LongCounter};
use util::statistics::counter::MonotoneNanoTime;
use util::heap::layout::heap_layout::VMMap;
use util::heap::layout::heap_layout::Mmapper;
use util::heap::layout::Mmapper as IMmapper;
use util::options::{Options, UnsafeOptionsWrapper};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use mmtk::MMTK;

// FIXME: Move somewhere more appropriate
#[cfg(feature = "jikesrvm")]
pub fn create_vm_space(vm_map: &'static VMMap, mmapper: &'static Mmapper, heap: &mut HeapMeta) -> ImmortalSpace {
    let boot_segment_bytes = BOOT_IMAGE_END - BOOT_IMAGE_DATA_START;
    debug_assert!(boot_segment_bytes > 0);

    let boot_segment_mb = unsafe{Address::from_usize(boot_segment_bytes)}
        .align_up(BYTES_IN_CHUNK).as_usize() >> LOG_BYTES_IN_MBYTE;

    ImmortalSpace::new("boot", false, VMRequest::fixed_size(boot_segment_mb), vm_map, mmapper, heap)
}

#[cfg(feature = "openjdk")]
pub fn create_vm_space(vm_map: &'static VMMap, mmapper: &'static Mmapper, heap: &mut HeapMeta) -> ImmortalSpace {
    // FIXME: Does OpenJDK care?
    ImmortalSpace::new("boot", false, VMRequest::fixed_size(0), vm_map, mmapper, heap)
}

pub trait Plan: Sized {
    type MutatorT: MutatorContext;
    type TraceLocalT: TraceLocal;
    type CollectorT: ParallelCollector;

    fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper, options: Arc<UnsafeOptionsWrapper>) -> Self;
    fn common(&self) -> &CommonPlan;
    fn mmapper(&self) -> &'static Mmapper {
        self.common().mmapper
    }
    fn options(&self) -> &Options {
        &self.common().options
    }
    // unsafe because this can only be called once by the init thread
    unsafe fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap);
    fn bind_mutator(&'static self, tls: OpaquePointer) -> *mut c_void;
    fn will_never_move(&self, object: ObjectReference) -> bool;
    // unsafe because only the primary collector thread can call this
    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase);

    fn is_initialized(&self) -> bool {
        self.common().initialized.load(Ordering::SeqCst)
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
            self.common().control_collector_context.request();
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
        info!("  [POLL] {}: {}", space.get_name(), message);
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

    fn get_total_pages(&self) -> usize {
        self.common().heap.get_total_pages()
    }

    fn get_pages_avail(&self) -> usize {
        self.get_total_pages() - self.get_pages_reserved()
    }

    fn get_collection_reserve(&self) -> usize {
        0
    }

    fn get_pages_used(&self) -> usize;

    fn is_emergency_collection(&self) -> bool {
        self.common().emergency_collection.load(Ordering::Relaxed)
    }

    fn get_free_pages(&self) -> usize { self.get_total_pages() - self.get_pages_used() }

    #[inline]
    fn stress_test_gc_required(&self) -> bool {
        let pages = self.common().vm_map.get_cumulative_committed_pages();
        trace!("pages={}", pages);

        if self.is_initialized()
            && (pages ^ self.common().last_stress_pages.load(Ordering::Relaxed)
            > self.options().stress_factor) {

            self.common().last_stress_pages.store(pages, Ordering::Relaxed);
            trace!("Doing stress GC");
            true
        } else {
            false
        }
    }

    fn is_internal_triggered_collection(&self) -> bool {
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

    fn handle_user_collection_request(&self, tls: OpaquePointer, force: bool) {
        if force || !self.options().ignore_system_g_c {
            self.common().user_triggered_collection.store(true, Ordering::Relaxed);
            self.common().control_collector_context.request();
            VMCollection::block_for_gc(tls);
        }
    }

    fn is_user_triggered_collection(&self) -> bool {
        self.common().user_triggered_collection.load(Ordering::Relaxed)
    }

    fn reset_collection_trigger(&self) {
        self.common().user_triggered_collection.store(false, Ordering::Relaxed)
    }

    fn determine_collection_attempts(&self) -> usize {
        if !self.common().allocation_success.load(Ordering::Relaxed) {
            self.common().max_collection_attempts.fetch_add(1, Ordering::Relaxed);
        } else {
            self.common().allocation_success.store(false, Ordering::Relaxed);
            self.common().max_collection_attempts.store(1, Ordering::Relaxed);
        }

        self.common().max_collection_attempts.load(Ordering::Relaxed)
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
        if self.common().gc_in_progress_proper() {
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

pub struct CommonPlan {
    pub vm_map: &'static VMMap,
    pub mmapper: &'static Mmapper,
    pub options: Arc<UnsafeOptionsWrapper>,
    pub heap: HeapMeta,

    pub initialized: AtomicBool,
    pub gc_status: Mutex<GcStatus>,
    pub last_stress_pages: AtomicUsize,
    pub stacks_prepared: AtomicBool,
    pub emergency_collection: AtomicBool,
    pub user_triggered_collection: AtomicBool,
    // Has an allocation succeeded since the emergency collection?
    pub allocation_success: AtomicBool,
    // Maximum number of failed attempts by a single thread
    pub max_collection_attempts: AtomicUsize,
    // Current collection attempt
    pub cur_collection_attempts: AtomicUsize,
    // Lock used for out of memory handling
    pub oom_lock: Mutex<()>,

    pub control_collector_context: ControllerCollectorContext,

    #[cfg(feature = "sanity")]
    pub inside_sanity: AtomicBool,
}

impl CommonPlan {
    pub fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper, options: Arc<UnsafeOptionsWrapper>, heap: HeapMeta) -> CommonPlan {
        CommonPlan {
            vm_map, mmapper, options, heap,
            initialized: AtomicBool::new(false),
            gc_status: Mutex::new(GcStatus::NotInGC),
            last_stress_pages: AtomicUsize::new(0),
            stacks_prepared: AtomicBool::new(false),
            emergency_collection: AtomicBool::new(false),
            user_triggered_collection: AtomicBool::new(false),
            allocation_success: AtomicBool::new(false),
            max_collection_attempts: AtomicUsize::new(0),
            cur_collection_attempts: AtomicUsize::new(0),
            oom_lock: Mutex::new(()),
            control_collector_context: ControllerCollectorContext::new(),

            #[cfg(feature = "sanity")]
            inside_sanity: AtomicBool::new(false),
        }
    }

    pub fn set_gc_status(&self, s: GcStatus) {
        let mut gc_status = self.gc_status.lock().unwrap();
        if *gc_status == GcStatus::NotInGC {
            self.stacks_prepared.store(false, Ordering::SeqCst);
            // FIXME stats
            STATS.lock().unwrap().start_gc();
        }
        *gc_status = s;
        if *gc_status == GcStatus::NotInGC {
            // FIXME stats
            if get_gathering_stats() {
                STATS.lock().unwrap().end_gc();
            }
        }
    }

    pub fn stacks_prepared(&self) -> bool {
        self.stacks_prepared.load(Ordering::SeqCst)
    }

    pub fn gc_in_progress(&self) -> bool {
        *self.gc_status.lock().unwrap() != GcStatus::NotInGC
    }

    pub fn gc_in_progress_proper(&self) -> bool {
        *self.gc_status.lock().unwrap() == GcStatus::GcProper
    }

    #[cfg(feature = "sanity")]
    pub fn enter_sanity(&self) {
        self.inside_sanity.store(true, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    pub fn leave_sanity(&self) {
        self.inside_sanity.store(false, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    pub fn is_in_sanity(&self) -> bool {
        self.inside_sanity.load(Ordering::Relaxed)
    }
}

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
