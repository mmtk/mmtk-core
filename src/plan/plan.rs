use super::controller_collector_context::ControllerCollectorContext;
use super::{MutatorContext, ParallelCollector, TraceLocal};
use crate::plan::phase::Phase;
use crate::plan::transitive_closure::TransitiveClosure;
use crate::policy::space::Space;
use crate::util::heap::layout::heap_layout::Mmapper;
use crate::util::heap::layout::heap_layout::VMMap;
use crate::util::heap::layout::Mmapper as IMmapper;
use crate::util::heap::HeapMeta;
use crate::util::heap::PageResource;
use crate::util::options::{Options, UnsafeOptionsWrapper};
use crate::util::statistics::stats::Stats;
use crate::util::Address;
use crate::util::ObjectReference;
use crate::util::OpaquePointer;
use crate::vm::VMBinding;
use crate::vm::{Collection, ObjectModel};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub trait Plan<VM: VMBinding>: Sized {
    type MutatorT: MutatorContext;
    type TraceLocalT: TraceLocal;
    type CollectorT: ParallelCollector<VM>;

    fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
    ) -> Self;
    fn common(&self) -> &CommonPlan<VM>;
    fn mmapper(&self) -> &'static Mmapper {
        self.common().mmapper
    }
    fn options(&self) -> &Options {
        &self.common().options
    }
    // unsafe because this can only be called once by the init thread
    fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap);
    fn bind_mutator(&'static self, tls: OpaquePointer) -> Box<Self::MutatorT>;
    fn will_never_move(&self, object: ObjectReference) -> bool;
    // unsafe because only the primary collector thread can call this
    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase);

    fn is_initialized(&self) -> bool {
        self.common().initialized.load(Ordering::SeqCst)
    }

    fn poll<PR: PageResource<VM>>(&self, space_full: bool, space: &'static PR::Space) -> bool {
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

        false
    }

    fn log_poll<PR: PageResource<VM>>(&self, space: &'static PR::Space, message: &'static str) {
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
    fn collection_required<PR: PageResource<VM>>(
        &self,
        space_full: bool,
        _space: &'static PR::Space,
    ) -> bool
    where
        Self: Sized,
    {
        let stress_force_gc = self.stress_test_gc_required();
        trace!(
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

    fn get_free_pages(&self) -> usize {
        self.get_total_pages() - self.get_pages_used()
    }

    #[inline]
    fn stress_test_gc_required(&self) -> bool {
        let pages = self.common().vm_map.get_cumulative_committed_pages();
        trace!("pages={}", pages);

        if self.is_initialized()
            && (pages ^ self.common().last_stress_pages.load(Ordering::Relaxed)
                > self.options().stress_factor)
        {
            self.common()
                .last_stress_pages
                .store(pages, Ordering::Relaxed);
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

    fn force_full_heap_collection(&self) {}

    fn is_valid_ref(&self, object: ObjectReference) -> bool;

    fn is_bad_ref(&self, object: ObjectReference) -> bool;

    fn handle_user_collection_request(&self, tls: OpaquePointer, force: bool) {
        if force || !self.options().ignore_system_g_c {
            self.common()
                .user_triggered_collection
                .store(true, Ordering::Relaxed);
            self.common().control_collector_context.request();
            VM::VMCollection::block_for_gc(tls);
        }
    }

    fn is_user_triggered_collection(&self) -> bool {
        self.common()
            .user_triggered_collection
            .load(Ordering::Relaxed)
    }

    fn reset_collection_trigger(&self) {
        self.common()
            .user_triggered_collection
            .store(false, Ordering::Relaxed)
    }

    fn determine_collection_attempts(&self) -> usize {
        if !self.common().allocation_success.load(Ordering::Relaxed) {
            self.common()
                .max_collection_attempts
                .fetch_add(1, Ordering::Relaxed);
        } else {
            self.common()
                .allocation_success
                .store(false, Ordering::Relaxed);
            self.common()
                .max_collection_attempts
                .store(1, Ordering::Relaxed);
        }

        self.common()
            .max_collection_attempts
            .load(Ordering::Relaxed)
    }

    fn is_mapped_object(&self, object: ObjectReference) -> bool {
        if object.is_null() {
            return false;
        }
        if !self.is_valid_ref(object) {
            return false;
        }
        if !self
            .mmapper()
            .address_is_mapped(VM::VMObjectModel::ref_to_address(object))
        {
            return false;
        }
        true
    }

    fn is_mapped_address(&self, address: Address) -> bool;

    fn modify_check(&self, object: ObjectReference) {
        if self.common().gc_in_progress_proper() && self.is_movable(object) {
            panic!(
                "GC modifying a potentially moving object via Java (i.e. not magic) obj= {}",
                object
            );
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

pub struct CommonPlan<VM: VMBinding> {
    pub vm_map: &'static VMMap,
    pub mmapper: &'static Mmapper,
    pub options: Arc<UnsafeOptionsWrapper>,

    #[cfg(feature = "vmspace")]
    pub unsync: UnsafeCell<CommonUnsync<VM>>,

    pub stats: Stats,
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

    pub control_collector_context: ControllerCollectorContext<VM>,

    #[cfg(feature = "sanity")]
    pub inside_sanity: AtomicBool,
}

#[cfg(feature = "vmspace")]
pub struct CommonUnsync<VM: VMBinding> {
    pub vm_space: Option<ImmortalSpace<VM>>,
}

#[cfg(feature = "vmspace")]
pub fn create_vm_space<VM: VMBinding>(
    vm_map: &'static VMMap,
    mmapper: &'static Mmapper,
    heap: &mut HeapMeta,
    boot_segment_bytes: usize,
) -> ImmortalSpace<VM> {
    //    let boot_segment_bytes = BOOT_IMAGE_END - BOOT_IMAGE_DATA_START;
    debug_assert!(boot_segment_bytes > 0);

    let boot_segment_mb =
        conversions::raw_align_up(boot_segment_bytes, BYTES_IN_CHUNK) >> LOG_BYTES_IN_MBYTE;

    ImmortalSpace::new(
        "boot",
        false,
        VMRequest::fixed_size(boot_segment_mb),
        vm_map,
        mmapper,
        heap,
    )
}

impl<VM: VMBinding> CommonPlan<VM> {
    #[allow(unused_mut)] // conditional compilation of vmspace
    pub fn new(
        vm_map: &'static VMMap,
        mmapper: &'static Mmapper,
        options: Arc<UnsafeOptionsWrapper>,
        mut heap: HeapMeta,
    ) -> CommonPlan<VM> {
        CommonPlan {
            vm_map,
            mmapper,

            #[cfg(feature = "vmspace")]
            unsync: UnsafeCell::new(CommonUnsync {
                vm_space: if options.vm_space {
                    Some(create_vm_space(
                        vm_map,
                        mmapper,
                        &mut heap,
                        options.vm_space_size,
                    ))
                } else {
                    None
                },
            }),

            options,
            heap,
            stats: Stats::new(),
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

    pub fn gc_init(&self, _heap_size: usize, _vm_map: &'static VMMap) {
        #[cfg(feature = "vmspace")]
        {
            let unsync = unsafe { &mut *self.unsync.get() };
            if unsync.vm_space.is_some() {
                unsync.vm_space.as_mut().unwrap().init(_vm_map);
            }
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

    pub fn is_valid_ref(&self, _object: ObjectReference) -> bool {
        #[cfg(feature = "vmspace")]
        {
            let unsync_c = unsafe { &mut *self.unsync.get() };
            if unsync_c.vm_space.is_some() && unsync_c.vm_space.as_ref().unwrap().in_space(_object)
            {
                return true;
            }
        }
        false
    }

    pub fn is_movable(&self, _object: ObjectReference) -> bool {
        #[cfg(feature = "vmspace")]
        {
            let unsync = unsafe { &*self.unsync.get() };
            if unsync.vm_space.is_some() && unsync.vm_space.as_ref().unwrap().in_space(_object) {
                return unsync.vm_space.as_ref().unwrap().is_movable();
            }
        }
        true
    }

    // Move into trace local
    pub fn is_live(&self, _object: ObjectReference) -> bool {
        #[cfg(feature = "vmspace")]
        {
            let unsync = unsafe { &*self.unsync.get() };
            if unsync.vm_space.is_some() && unsync.vm_space.as_ref().unwrap().in_space(_object) {
                return true;
            }
        }
        panic!("Invalid space")
    }

    pub fn trace_object<T: TransitiveClosure>(
        &self,
        _trace: &mut T,
        _object: ObjectReference,
    ) -> ObjectReference {
        #[cfg(feature = "vmspace")]
        {
            let unsync = unsafe { &*self.unsync.get() };
            if unsync.vm_space.is_some() && unsync.vm_space.as_ref().unwrap().in_space(_object) {
                trace!("trace_object: object in boot space");
                return unsync
                    .vm_space
                    .as_ref()
                    .unwrap()
                    .trace_object(_trace, _object);
            }
        }
        panic!("No special case for space in trace_object");
    }

    pub fn is_mapped_address(&self, _address: Address) -> bool {
        #[cfg(feature = "vmspace")]
        {
            let unsync_c = unsafe { &mut *self.unsync.get() };
            if unsafe {
                unsync_c.vm_space.is_some()
                    && unsync_c
                        .vm_space
                        .as_ref()
                        .unwrap()
                        .in_space(_address.to_object_reference())
            } {
                return self.mmapper.address_is_mapped(_address);
            }
        }

        false
    }

    pub unsafe fn collection_phase(&self, _tls: OpaquePointer, _phase: &Phase) {
        #[cfg(feature = "vmspace")]
        {
            let unsync = unsafe { &mut *self.unsync.get() };
            match phase {
                Phase::Prepare => {
                    if unsync.vm_space.is_some() {
                        unsync.vm_space.as_mut().unwrap().prepare();
                    }
                }
                &Phase::Release => {
                    if unsync.vm_space.is_some() {
                        unsync.vm_space.as_mut().unwrap().release();
                    }
                }
                _ => {}
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
