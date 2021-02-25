//! The global part of a plan implementation.

use super::controller_collector_context::ControllerCollectorContext;
use super::PlanConstraints;
use crate::mmtk::MMTK;
use crate::plan::transitive_closure::TransitiveClosure;
use crate::plan::Mutator;
use crate::policy::immortalspace::ImmortalSpace;
use crate::policy::largeobjectspace::LargeObjectSpace;
use crate::policy::space::Space;
use crate::scheduler::gc_work::ProcessEdgesWork;
use crate::scheduler::*;
use crate::util::alloc::allocators::AllocatorSelector;
#[cfg(feature = "analysis")]
use crate::util::analysis::AnalysisManager;
use crate::util::conversions::bytes_to_pages;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::map::Map;
use crate::util::heap::HeapMeta;
use crate::util::heap::VMRequest;
use crate::util::options::PlanSelector;
use crate::util::options::{Options, UnsafeOptionsWrapper};
use crate::util::statistics::stats::Stats;
use crate::util::OpaquePointer;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use downcast_rs::Downcast;
use enum_map::EnumMap;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A GC worker's context for copying GCs.
/// Each GC plan should provide their implementation of a CopyContext.
/// For non-copying GC, NoCopy can be used.
pub trait CopyContext: 'static + Sync + Send {
    type VM: VMBinding;
    fn constraints(&self) -> &'static PlanConstraints;
    fn init(&mut self, tls: OpaquePointer);
    fn prepare(&mut self);
    fn release(&mut self);
    fn alloc_copy(
        &mut self,
        original: ObjectReference,
        bytes: usize,
        align: usize,
        offset: isize,
        semantics: AllocationSemantics,
    ) -> Address;
    fn post_copy(
        &mut self,
        _obj: ObjectReference,
        _tib: Address,
        _bytes: usize,
        _semantics: AllocationSemantics,
    ) {
    }
    fn copy_check_allocator(
        &self,
        _from: ObjectReference,
        bytes: usize,
        align: usize,
        semantics: AllocationSemantics,
    ) -> AllocationSemantics {
        let large = crate::util::alloc::allocator::get_maximum_aligned_size::<Self::VM>(
            bytes,
            align,
            Self::VM::MIN_ALIGNMENT,
        ) > self.constraints().max_non_los_copy_bytes;
        if large {
            AllocationSemantics::Los
        } else {
            semantics
        }
    }
}

pub struct NoCopy<VM: VMBinding>(PhantomData<VM>);

impl<VM: VMBinding> CopyContext for NoCopy<VM> {
    type VM = VM;

    fn init(&mut self, _tls: OpaquePointer) {}
    fn constraints(&self) -> &'static PlanConstraints {
        unreachable!()
    }
    fn prepare(&mut self) {}
    fn release(&mut self) {}
    fn alloc_copy(
        &mut self,
        _original: ObjectReference,
        _bytes: usize,
        _align: usize,
        _offset: isize,
        _semantics: AllocationSemantics,
    ) -> Address {
        unreachable!()
    }
}

impl<VM: VMBinding> NoCopy<VM> {
    pub fn new(_mmtk: &'static MMTK<VM>) -> Self {
        Self(PhantomData)
    }
}

impl<VM: VMBinding> WorkerLocal for NoCopy<VM> {
    fn init(&mut self, tls: OpaquePointer) {
        CopyContext::init(self, tls);
    }
}

pub fn create_mutator<VM: VMBinding>(
    tls: OpaquePointer,
    mmtk: &'static MMTK<VM>,
) -> Box<Mutator<VM>> {
    Box::new(match mmtk.options.plan {
        PlanSelector::NoGC => crate::plan::nogc::mutator::create_nogc_mutator(tls, &*mmtk.plan),
        PlanSelector::SemiSpace => {
            crate::plan::semispace::mutator::create_ss_mutator(tls, &*mmtk.plan)
        }
        PlanSelector::GenCopy => crate::plan::gencopy::mutator::create_gencopy_mutator(tls, mmtk),
    })
}

pub fn create_plan<VM: VMBinding>(
    plan: PlanSelector,
    vm_map: &'static VMMap,
    mmapper: &'static Mmapper,
    options: Arc<UnsafeOptionsWrapper>,
    scheduler: &'static MMTkScheduler<VM>,
) -> Box<dyn Plan<VM = VM>> {
    match plan {
        PlanSelector::NoGC => Box::new(crate::plan::nogc::NoGC::new(
            vm_map, mmapper, options, scheduler,
        )),
        PlanSelector::SemiSpace => Box::new(crate::plan::semispace::SemiSpace::new(
            vm_map, mmapper, options, scheduler,
        )),
        PlanSelector::GenCopy => Box::new(crate::plan::gencopy::GenCopy::new(
            vm_map, mmapper, options, scheduler,
        )),
    }
}

/// A plan describes the global core functionality for all memory management schemes.
/// All global MMTk plans should implement this trait.
///
/// The global instance defines and manages static resources
/// (such as memory and virtual memory resources).
pub trait Plan: 'static + Sync + Send + Downcast {
    type VM: VMBinding;

    fn constraints(&self) -> &'static PlanConstraints;
    fn create_worker_local(
        &self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr;
    fn base(&self) -> &BasePlan<Self::VM>;
    fn schedule_collection(&'static self, _scheduler: &MMTkScheduler<Self::VM>);
    fn common(&self) -> &CommonPlan<Self::VM> {
        panic!("Common Plan not handled!")
    }
    fn mmapper(&self) -> &'static Mmapper {
        self.base().mmapper
    }
    fn options(&self) -> &Options {
        &self.base().options
    }

    // unsafe because this can only be called once by the init thread
    fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<Self::VM>>,
    );

    fn get_allocator_mapping(&self) -> &'static EnumMap<AllocationSemantics, AllocatorSelector>;

    fn in_nursery(&self) -> bool {
        false
    }

    #[cfg(feature = "sanity")]
    fn enter_sanity(&self) {
        self.base().inside_sanity.store(true, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    fn leave_sanity(&self) {
        self.base().inside_sanity.store(false, Ordering::Relaxed)
    }

    #[cfg(feature = "sanity")]
    fn is_in_sanity(&self) -> bool {
        self.base().inside_sanity.load(Ordering::Relaxed)
    }

    fn is_initialized(&self) -> bool {
        self.base().initialized.load(Ordering::SeqCst)
    }

    fn prepare(&self, tls: OpaquePointer);
    fn release(&self, tls: OpaquePointer);

    fn poll(&self, space_full: bool, space: &dyn Space<Self::VM>) -> bool {
        if self.collection_required(space_full, space) {
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
            self.log_poll(space, "Triggering collection");
            self.base().control_collector_context.request();
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

        false
    }

    fn log_poll(&self, space: &dyn Space<Self::VM>, message: &'static str) {
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
    fn collection_required(&self, space_full: bool, _space: &dyn Space<Self::VM>) -> bool {
        let stress_force_gc = self.stress_test_gc_required();
        debug!(
            "self.get_pages_reserved()={}, self.get_total_pages()={}",
            self.get_pages_reserved(),
            self.get_total_pages()
        );
        let heap_full = self.get_pages_reserved() > self.get_total_pages();

        space_full || stress_force_gc || heap_full
    }

    fn get_pages_reserved(&self) -> usize {
        self.get_pages_used() + self.get_collection_reserve()
    }

    fn get_total_pages(&self) -> usize {
        self.base().heap.get_total_pages()
    }

    fn get_pages_avail(&self) -> usize {
        self.get_total_pages() - self.get_pages_reserved()
    }

    fn get_collection_reserve(&self) -> usize {
        0
    }

    fn get_pages_used(&self) -> usize;

    fn is_emergency_collection(&self) -> bool {
        self.base().emergency_collection.load(Ordering::Relaxed)
    }

    fn get_free_pages(&self) -> usize {
        self.get_total_pages() - self.get_pages_used()
    }

    #[inline]
    fn stress_test_gc_required(&self) -> bool {
        let stress_factor = self.base().options.stress_factor;
        if self.is_initialized()
            && (self.base().allocation_bytes.load(Ordering::SeqCst) > stress_factor)
        {
            trace!(
                "Stress GC: allocation_bytes = {}, stress_factor = {}",
                self.base().allocation_bytes.load(Ordering::Relaxed),
                stress_factor
            );
            trace!("Doing stress GC");
            self.base().allocation_bytes.store(0, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    fn handle_user_collection_request(&self, tls: OpaquePointer, force: bool) {
        if force || !self.options().ignore_system_g_c {
            info!("User triggerring collection");
            self.base()
                .user_triggered_collection
                .store(true, Ordering::Relaxed);
            self.base().control_collector_context.request();
            <Self::VM as VMBinding>::VMCollection::block_for_gc(tls);
        }
    }

    fn reset_collection_trigger(&self) {
        self.base()
            .user_triggered_collection
            .store(false, Ordering::Relaxed)
    }

    fn modify_check(&self, object: ObjectReference) {
        if self.base().gc_in_progress_proper() && object.is_movable() {
            panic!(
                "GC modifying a potentially moving object via Java (i.e. not magic) obj= {}",
                object
            );
        }
    }

    fn global_side_metadata_per_chunk(&self) -> usize {
        0
    }
}

impl_downcast!(Plan assoc VM);

#[derive(PartialEq)]
pub enum GcStatus {
    NotInGC,
    GcPrepare,
    GcProper,
}

/**
BasePlan should contain all plan-related state and functions that are _fundamental_ to _all_ plans.  These include VM-specific (but not plan-specific) features such as a code space or vm space, which are fundamental to all plans for a given VM.  Features that are common to _many_ (but not intrinsically _all_) plans should instead be included in CommonPlan.
*/
pub struct BasePlan<VM: VMBinding> {
    // Whether MMTk is now ready for collection. This is set to true when enable_collection() is called.
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
    pub control_collector_context: ControllerCollectorContext<VM>,
    pub stats: Stats,
    mmapper: &'static Mmapper,
    pub vm_map: &'static VMMap,
    pub options: Arc<UnsafeOptionsWrapper>,
    pub heap: HeapMeta,
    #[cfg(feature = "base_spaces")]
    pub unsync: UnsafeCell<BaseUnsync<VM>>,
    #[cfg(feature = "sanity")]
    pub inside_sanity: AtomicBool,
    // A counter for per-mutator stack scanning
    pub scanned_stacks: AtomicUsize,
    pub mutator_iterator_lock: Mutex<()>,
    // A counter that keeps tracks of the number of bytes allocated since last stress test
    pub allocation_bytes: AtomicUsize,
    // Wrapper around analysis counters
    #[cfg(feature = "analysis")]
    pub analysis_manager: AnalysisManager<VM>,
}

#[cfg(feature = "base_spaces")]
pub struct BaseUnsync<VM: VMBinding> {
    #[cfg(feature = "code_space")]
    pub code_space: ImmortalSpace<VM>,
    #[cfg(feature = "ro_space")]
    pub ro_space: ImmortalSpace<VM>,
    #[cfg(feature = "vm_space")]
    pub vm_space: ImmortalSpace<VM>,
}

#[cfg(feature = "vm_space")]
pub fn create_vm_space<VM: VMBinding>(
    vm_map: &'static VMMap,
    mmapper: &'static Mmapper,
    heap: &mut HeapMeta,
    boot_segment_bytes: usize,
    constraints: &'static PlanConstraints,
) -> ImmortalSpace<VM> {
    use crate::util::constants::LOG_BYTES_IN_MBYTE;
    //    let boot_segment_bytes = BOOT_IMAGE_END - BOOT_IMAGE_DATA_START;
    debug_assert!(boot_segment_bytes > 0);

    use crate::util::conversions::raw_align_up;
    use crate::util::heap::layout::vm_layout_constants::BYTES_IN_CHUNK;
    let boot_segment_mb = raw_align_up(boot_segment_bytes, BYTES_IN_CHUNK) >> LOG_BYTES_IN_MBYTE;

    ImmortalSpace::new(
        "boot",
        false,
        VMRequest::fixed_size(boot_segment_mb),
        vm_map,
        mmapper,
        heap,
        constraints,
    )
}

impl<VM: VMBinding> BasePlan<VM> {
    #[allow(unused_mut)] // 'heap' only needs to be mutable for certain features
    #[allow(unused_variables)] // 'constraints' is only needed for certain features
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        mut heap: HeapMeta,
        constraints: &'static PlanConstraints,
    ) -> BasePlan<VM> {
        let stats = Stats::new();
        // Initializing the analysis manager and routines
        #[cfg(feature = "analysis")]
        let analysis_manager = AnalysisManager::new(&stats);
        BasePlan {
            #[cfg(feature = "base_spaces")]
            unsync: UnsafeCell::new(BaseUnsync {
                #[cfg(feature = "code_space")]
                code_space: ImmortalSpace::new(
                    "code_space",
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                    constraints,
                ),
                #[cfg(feature = "ro_space")]
                ro_space: ImmortalSpace::new(
                    "ro_space",
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                    constraints,
                ),
                #[cfg(feature = "vm_space")]
                vm_space: create_vm_space(
                    vm_map,
                    mmapper,
                    &mut heap,
                    options.vm_space_size,
                    constraints,
                ),
            }),
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
            stats,
            mmapper,
            heap,
            vm_map,
            options,
            #[cfg(feature = "sanity")]
            inside_sanity: AtomicBool::new(false),
            scanned_stacks: AtomicUsize::new(0),
            mutator_iterator_lock: Mutex::new(()),
            allocation_bytes: AtomicUsize::new(0),
            #[cfg(feature = "analysis")]
            analysis_manager,
        }
    }

    pub fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        vm_map.boot();
        vm_map.finalize_static_space_map(
            self.heap.get_discontig_start(),
            self.heap.get_discontig_end(),
        );
        self.heap
            .total_pages
            .store(bytes_to_pages(heap_size), Ordering::Relaxed);
        self.control_collector_context.init(scheduler);

        #[cfg(feature = "base_spaces")]
        {
            let unsync = unsafe { &mut *self.unsync.get() };
            #[cfg(feature = "code_space")]
            unsync.code_space.init(vm_map);
            #[cfg(feature = "ro_space")]
            unsync.ro_space.init(vm_map);
            #[cfg(feature = "vm_space")]
            {
                unsync.vm_space.init(vm_map);
                unsync.vm_space.ensure_mapped();
            }
        }
    }

    #[cfg(feature = "base_spaces")]
    pub fn get_pages_used(&self) -> usize {
        let mut pages = 0;
        let unsync = unsafe { &mut *self.unsync.get() };

        #[cfg(feature = "code_space")]
        {
            pages += unsync.code_space.reserved_pages();
        }
        #[cfg(feature = "ro_space")]
        {
            pages += unsync.ro_space.reserved_pages();
        }

        // The VM space may be used as an immutable boot image, in which case, we should not count
        // it as part of the heap size.
        // #[cfg(feature = "vm_space")]
        // {
        //     pages += unsync.vm_space.reserved_pages();
        // }
        pages
    }

    #[cfg(not(feature = "base_spaces"))]
    pub fn get_pages_used(&self) -> usize {
        0
    }

    pub fn trace_object<T: TransitiveClosure, C: CopyContext>(
        &self,
        _trace: &mut T,
        _object: ObjectReference,
    ) -> ObjectReference {
        #[cfg(feature = "base_spaces")]
        {
            let unsync = unsafe { &*self.unsync.get() };

            #[cfg(feature = "code_space")]
            {
                if unsync.code_space.in_space(_object) {
                    trace!("trace_object: object in code space");
                    return unsync.code_space.trace_object::<T>(_trace, _object);
                }
            }

            #[cfg(feature = "ro_space")]
            {
                if unsync.ro_space.in_space(_object) {
                    trace!("trace_object: object in ro_space space");
                    return unsync.ro_space.trace_object(_trace, _object);
                }
            }

            #[cfg(feature = "vm_space")]
            {
                if unsync.vm_space.in_space(_object) {
                    trace!("trace_object: object in boot space");
                    return unsync.vm_space.trace_object(_trace, _object);
                }
            }
        }
        panic!("No special case for space in trace_object({:?})", _object);
    }

    pub fn prepare(&self, _tls: OpaquePointer, _primary: bool) {
        #[cfg(feature = "base_spaces")]
        let unsync = unsafe { &mut *self.unsync.get() };
        #[cfg(feature = "code_space")]
        unsync.code_space.prepare();
        #[cfg(feature = "ro_space")]
        unsync.ro_space.prepare();
        #[cfg(feature = "vm_space")]
        unsync.vm_space.prepare();
    }

    pub fn release(&self, _tls: OpaquePointer, _primary: bool) {
        #[cfg(feature = "base_spaces")]
        let unsync = unsafe { &mut *self.unsync.get() };
        #[cfg(feature = "code_space")]
        unsync.code_space.release();
        #[cfg(feature = "ro_space")]
        unsync.ro_space.release();
        #[cfg(feature = "vm_space")]
        unsync.vm_space.release();
    }

    pub fn set_collection_kind(&self) {
        self.cur_collection_attempts.store(
            if self.is_user_triggered_collection() {
                1
            } else {
                self.determine_collection_attempts()
            },
            Ordering::Relaxed,
        );

        let emergency_collection = !self.is_internal_triggered_collection()
            && self.last_collection_was_exhaustive()
            && self.cur_collection_attempts.load(Ordering::Relaxed) > 1;
        self.emergency_collection
            .store(emergency_collection, Ordering::Relaxed);

        if emergency_collection {
            self.force_full_heap_collection();
        }
    }

    pub fn set_gc_status(&self, s: GcStatus) {
        let mut gc_status = self.gc_status.lock().unwrap();
        if *gc_status == GcStatus::NotInGC {
            self.stacks_prepared.store(false, Ordering::SeqCst);
            // FIXME stats
            self.stats.start_gc();
        }
        *gc_status = s;
        if *gc_status == GcStatus::NotInGC {
            // FIXME stats
            if self.stats.get_gathering_stats() {
                self.stats.end_gc();
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

    fn is_user_triggered_collection(&self) -> bool {
        self.user_triggered_collection.load(Ordering::Relaxed)
    }

    fn determine_collection_attempts(&self) -> usize {
        if !self.allocation_success.load(Ordering::Relaxed) {
            self.max_collection_attempts.fetch_add(1, Ordering::Relaxed);
        } else {
            self.allocation_success.store(false, Ordering::Relaxed);
            self.max_collection_attempts.store(1, Ordering::Relaxed);
        }

        self.max_collection_attempts.load(Ordering::Relaxed)
    }

    fn is_internal_triggered_collection(&self) -> bool {
        // FIXME
        false
    }

    fn last_collection_was_exhaustive(&self) -> bool {
        true
    }

    fn force_full_heap_collection(&self) {}

    pub fn increase_allocation_bytes_by(&self, size: usize) {
        let old_allocation_bytes = self.allocation_bytes.fetch_add(size, Ordering::SeqCst);
        trace!(
            "Stress GC: old_allocation_bytes = {}, size = {}, allocation_bytes = {}",
            old_allocation_bytes,
            size,
            self.allocation_bytes.load(Ordering::Relaxed),
        );
    }
}

/**
CommonPlan is for representing state and features used by _many_ plans, but that are not fundamental to _all_ plans.  Examples include the Large Object Space and an Immortal space.  Features that are fundamental to _all_ plans must be included in BasePlan.
*/
pub struct CommonPlan<VM: VMBinding> {
    pub unsync: UnsafeCell<CommonUnsync<VM>>,
    pub base: BasePlan<VM>,
}

pub struct CommonUnsync<VM: VMBinding> {
    pub immortal: ImmortalSpace<VM>,
    pub los: LargeObjectSpace<VM>,
}

impl<VM: VMBinding> CommonPlan<VM> {
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        mut heap: HeapMeta,
        constraints: &'static PlanConstraints,
    ) -> CommonPlan<VM> {
        CommonPlan {
            unsync: UnsafeCell::new(CommonUnsync {
                immortal: ImmortalSpace::new(
                    "immortal",
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                    constraints,
                ),
                los: LargeObjectSpace::new(
                    "los",
                    true,
                    VMRequest::discontiguous(),
                    vm_map,
                    mmapper,
                    &mut heap,
                    constraints,
                ),
            }),
            base: BasePlan::new(vm_map, mmapper, options, heap, constraints),
        }
    }

    pub fn gc_init(
        &mut self,
        heap_size: usize,
        vm_map: &'static VMMap,
        scheduler: &Arc<MMTkScheduler<VM>>,
    ) {
        self.base.gc_init(heap_size, vm_map, scheduler);
        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.immortal.init(vm_map);
        unsync.los.init(vm_map);
    }

    pub fn get_pages_used(&self) -> usize {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.immortal.reserved_pages() + unsync.los.reserved_pages() + self.base.get_pages_used()
    }

    pub fn trace_object<T: TransitiveClosure, C: CopyContext>(
        &self,
        trace: &mut T,
        object: ObjectReference,
    ) -> ObjectReference {
        let unsync = unsafe { &*self.unsync.get() };

        if unsync.immortal.in_space(object) {
            trace!("trace_object: object in immortal space");
            return unsync.immortal.trace_object(trace, object);
        }
        if unsync.los.in_space(object) {
            trace!("trace_object: object in los");
            return unsync.los.trace_object(trace, object);
        }
        self.base.trace_object::<T, C>(trace, object)
    }

    pub fn prepare(&self, tls: OpaquePointer, primary: bool) {
        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.immortal.prepare();
        unsync.los.prepare(primary);
        self.base.prepare(tls, primary)
    }

    pub fn release(&self, tls: OpaquePointer, primary: bool) {
        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.immortal.release();
        unsync.los.release(primary);
        self.base.release(tls, primary)
    }

    pub fn schedule_common<E: ProcessEdgesWork<VM = VM>>(
        &self,
        constraints: &'static PlanConstraints,
        scheduler: &MMTkScheduler<VM>,
    ) {
        // Schedule finalization
        if !self.base.options.no_finalizer {
            use crate::util::finalizable_processor::{Finalization, ForwardFinalization};
            // finalization
            scheduler.work_buckets[WorkBucketStage::RefClosure].add(Finalization::<E>::new());
            // forward refs
            if constraints.needs_forward_after_liveness {
                scheduler.work_buckets[WorkBucketStage::RefForwarding]
                    .add(ForwardFinalization::<E>::new());
            }
        }
    }

    pub fn stacks_prepared(&self) -> bool {
        self.base.stacks_prepared()
    }

    pub fn get_immortal(&self) -> &'static ImmortalSpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.immortal
    }

    pub fn get_los(&self) -> &'static LargeObjectSpace<VM> {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.los
    }
}

use enum_map::Enum;
/// Allocation semantics that MMTk provides.
/// Each allocation request requires a desired semantic for the object to allocate.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Enum)]
pub enum AllocationSemantics {
    Default = 0,
    Immortal = 1,
    Los = 2,
    Code = 3,
    ReadOnly = 4,
}
