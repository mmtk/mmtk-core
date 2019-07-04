use ::policy::space::Space;

use super::SSMutator;
use super::SSTraceLocal;
use super::SSCollector;

use ::plan::controller_collector_context::ControllerCollectorContext;

use ::plan::plan;
use ::plan::Plan;
use ::plan::Allocator;
use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::policy::largeobjectspace::LargeObjectSpace;
use ::plan::Phase;
use ::plan::trace::Trace;
use ::util::ObjectReference;
use ::util::alloc::allocator::determine_collection_attempts;
use ::util::heap::layout::heap_layout::MMAPPER;
use ::util::heap::layout::Mmapper;
use ::util::Address;
use ::util::heap::PageResource;
use ::util::heap::VMRequest;
use ::plan::phase;

use ::util::constants::LOG_BYTES_IN_PAGE;

use libc::{c_void, memset};
use std::cell::UnsafeCell;
use std::sync::atomic::{self, AtomicBool, AtomicUsize, Ordering};

use ::vm::{Scanning, VMScanning};
use std::thread;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;
use plan::plan::EMERGENCY_COLLECTION;
use ::util::queue::SharedQueue;
use super::VERBOSE;

use ::util::heap::layout::vm_layout_constants::{HEAP_START, HEAP_END};

pub type SelectedPlan = SemiSpace;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;
const NURSERY_VM_FRACTION: f32 = 0.15;
const WORST_CASE_COPY_EXPANSION: f32 = 1.5;

lazy_static! {
    pub static ref PLAN: SemiSpace = SemiSpace::new();

    pub static ref NURSERY_FULL_COLLECTION: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Complex, plan::INIT_PHASE.clone()),
        (phase::Schedule::Complex, plan::ROOT_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, plan::REF_TYPE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, plan::COMPLETE_CLOSURE_PHASE.clone()),
        super::validate::schedule_validation_phase(),
        (phase::Schedule::Complex, plan::FINISH_PHASE.clone()),
    ], 0);
}

pub struct SemiSpace {
    pub unsync: UnsafeCell<SemiSpaceUnsync>,
    pub ss_trace: Trace,
}

pub struct SemiSpaceUnsync {
    pub hi: bool,
    pub vm_space: ImmortalSpace,
    pub nursery_space: CopySpace,
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    pub versatile_space: ImmortalSpace,
    pub los: LargeObjectSpace,
    pub gc_full_heap: bool,
    pub next_gc_full_heap: bool,
    pub remset_pool: SharedQueue<Address>,
    // FIXME: This should be inside HeapGrowthManager
    total_pages: usize,
    collection_attempt: usize,
}

unsafe impl Sync for SemiSpace {}

impl Plan for SemiSpace {
    type MutatorT = SSMutator;
    type TraceLocalT = SSTraceLocal;
    type CollectorT = SSCollector;

    fn new() -> Self {
        SemiSpace {
            unsync: UnsafeCell::new(SemiSpaceUnsync {
                hi: false,
                vm_space: create_vm_space(),
                nursery_space: CopySpace::new("nursery", false, true, VMRequest::high_fraction(NURSERY_VM_FRACTION)),
                copyspace0: CopySpace::new("copyspace0", false, true, VMRequest::discontiguous()),
                copyspace1: CopySpace::new("copyspace1", true, true, VMRequest::discontiguous()),
                versatile_space: ImmortalSpace::new("versatile_space", true, VMRequest::discontiguous()),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous()),
                total_pages: 0,
                collection_attempt: 0,
                gc_full_heap: false,
                next_gc_full_heap: false,
                remset_pool: SharedQueue::new(),
            }),
            ss_trace: Trace::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        ::util::heap::layout::heap_layout::VM_MAP.finalize_static_space_map();
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        unsync.vm_space.init();
        unsync.nursery_space.init();
        unsync.copyspace0.init();
        unsync.copyspace1.init();
        unsync.versatile_space.init();
        unsync.los.init();

        // These VMs require that the controller thread is started by the VM itself.
        // (Usually because it calls into VM code that accesses the TLS.)
        if !(cfg!(feature = "jikesrvm") || cfg!(feature = "openjdk")) {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(0 as *mut c_void)
            });
        }
    }

    fn bind_mutator(&self, tls: *mut c_void) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        Box::into_raw(Box::new(SSMutator::new(tls, self.nursery_space(),
                                              &unsync.versatile_space, &unsync.los))) as *mut c_void
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };

        if self.nursery_space().in_space(object) || self.tospace().in_space(object) || self.fromspace().in_space(object) {
            return false;
        }

        if unsync.versatile_space.in_space(object) || unsync.los.in_space(object) {
            return true;
        }

        // this preserves correctness over efficiency
        false
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.versatile_space.in_space(object) {
            return true;
        }
        if unsync.vm_space.in_space(object) {
            return true;
        }
        if self.nursery_space().in_space(object) {
            return true;
        }
        if self.tospace().in_space(object) {
            return true;
        }
        if unsync.los.in_space(object) {
            return true;
        }
        return false;
    }

    fn collection_required<PR: PageResource>(&self, space_full: bool, space: &'static PR::Space) -> bool where Self: Sized {
        if VERBOSE && space_full {
            println!("Space {} is full", space.common().name);
        }

        if space_full && (space as *const PR::Space as usize) != (self.nursery_space() as *const CopySpace as usize) {
            let unsync = unsafe { &mut *self.unsync.get() };
            if VERBOSE {
                println!("next_gc_full_heap = true");
            }
            unsync.next_gc_full_heap = true;
        }

        let stress_force_gc = self.stress_test_gc_required();
        trace!("self.get_pages_reserved()={}, self.get_total_pages()={}",
               self.get_pages_reserved(), self.get_total_pages());
        let heap_full = self.get_pages_reserved() > self.get_total_pages();

        if VERBOSE && heap_full {
            println!("Heap is full");
        }
        space_full || stress_force_gc || heap_full
    }

    fn force_full_heap_collection(&self) {
        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.next_gc_full_heap = true;
    }

    unsafe fn collection_phase(&self, tls: *mut c_void, phase: &Phase) {
        if VERBOSE {
            println!("Global {:?}", phase);
        }
        let unsync = &mut *self.unsync.get();

        match phase {
            &Phase::SetCollectionKind => {
                let unsync = &mut *self.unsync.get();
                unsync.collection_attempt = if <SelectedPlan as Plan>::is_user_triggered_collection() {
                    1 } else { determine_collection_attempts() };

                let emergency_collection = !<SelectedPlan as Plan>::is_internal_triggered_collection()
                    && self.last_collection_was_exhaustive() && unsync.collection_attempt > 1;
                EMERGENCY_COLLECTION.store(emergency_collection, Ordering::Relaxed);

                if emergency_collection {
                    // println!("User triggered collection: {:?}", <SelectedPlan as Plan>::is_user_triggered_collection());
                    self.force_full_heap_collection();
                }

                unsync.gc_full_heap = self.requires_full_heap_collection();

                if VERBOSE {
                    if unsync.gc_full_heap {
                        // panic!("Not here yet");
                        println!("Full GC");
                    } else {
                        println!("Nursery GC");
                    }
                }

                if VERBOSE {
                    unsync.nursery_space.print_vm_map();
                    unsync.copyspace0.print_vm_map();
                    unsync.copyspace1.print_vm_map();
                    unsync.versatile_space.print_vm_map();
                    unsync.vm_space.print_vm_map();
                }
            }
            &Phase::Initiate => {
                plan::set_gc_status(plan::GcStatus::GcPrepare);
            }
            &Phase::PrepareStacks => {
                plan::STACKS_PREPARED.store(true, atomic::Ordering::SeqCst);
            }
            &Phase::Prepare => {
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                unsync.nursery_space.prepare(true);
                if unsync.gc_full_heap {
                    unsync.hi = !unsync.hi; // flip the semi-spaces
                    // prepare each of the collected regions
                    unsync.copyspace0.prepare(unsync.hi);
                    unsync.copyspace1.prepare(!unsync.hi);
                    unsync.versatile_space.prepare();
                    unsync.vm_space.prepare();
                    unsync.los.prepare(true);
                    unsync.remset_pool.clear();
                }
            }
            &Phase::StackRoots => {
                VMScanning::notify_initial_thread_scan_complete(!self.gc_full_heap, tls);
                plan::set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Roots => {
                VMScanning::reset_thread_counter();
                plan::set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Closure => {}
            &Phase::Release => {
                debug_assert!(self.remset_pool.is_empty());
                // release the collected region
                unsync.nursery_space.release();
                if unsync.gc_full_heap {
                    if unsync.hi {
                        unsync.copyspace0.release();
                    } else {
                        unsync.copyspace1.release();
                    }
                    unsync.versatile_space.release();
                    unsync.vm_space.release();
                    unsync.los.release(true);
                }
                { &mut *(self as *const Self as usize as *mut Self) }.next_gc_full_heap = false;//(self.get_pages_avail() < Options.nurserySize.getMinNursery());
            }
            &Phase::ValidatePrepare => {
                super::validate::prepare();
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                unsync.remset_pool.clear();
            }
            &Phase::ValidateRelease => {
                super::validate::release();
            }
            &Phase::Complete => {
                if VERBOSE {
                    unsync.nursery_space.print_vm_map();
                    unsync.copyspace0.print_vm_map();
                    unsync.copyspace1.print_vm_map();
                    unsync.versatile_space.print_vm_map();
                    unsync.los.print_vm_map();
                    unsync.vm_space.print_vm_map();
                }
                debug_assert!(self.remset_pool.is_empty());
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                plan::set_gc_status(plan::GcStatus::NotInGC);
            }
            _ => {
                panic!("Global phase not handled!")
            }
        }
    }

    fn get_total_pages(&self) -> usize {
        unsafe{(&*self.unsync.get()).total_pages}
    }

    fn get_collection_reserve(&self) -> usize {
        let unsync = unsafe{&*self.unsync.get()};
        self.tospace().reserved_pages() + self.nursery_space().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe{&*self.unsync.get()};
        self.nursery_space().reserved_pages() + self.tospace().reserved_pages() + unsync.versatile_space.reserved_pages() + unsync.los.reserved_pages()
    }

    fn is_bad_ref(&self, object: ObjectReference) -> bool {
        self.fromspace().in_space(object)
    }

    fn is_movable(&self, object: ObjectReference) -> bool {
        if self.vm_space.in_space(object) {
            return self.vm_space.is_movable();
        }
        if self.nursery_space.in_space(object) {
            return self.nursery_space.is_movable();
        }
        if self.copyspace0.in_space(object) {
            return self.copyspace0.is_movable();
        }
        if self.copyspace1.in_space(object) {
            return self.copyspace1.is_movable();
        }
        if self.versatile_space.in_space(object) {
            return self.versatile_space.is_movable();
        }
        if self.los.in_space(object) {
            return self.los.is_movable();
        }
        return true;
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        if unsafe {
            self.vm_space.in_space(address.to_object_reference())  ||
            self.versatile_space.in_space(address.to_object_reference()) ||
            self.nursery_space.in_space(address.to_object_reference()) ||
            self.copyspace0.in_space(address.to_object_reference()) ||
            self.copyspace1.in_space(address.to_object_reference()) ||
            self.los.in_space(address.to_object_reference())
        } {
            return MMAPPER.address_is_mapped(address);
        } else {
            return false;
        }
    }
}

impl SemiSpace {
    fn requires_full_heap_collection(&self) -> bool {
        if <SelectedPlan as Plan>::is_user_triggered_collection() && false {
            if VERBOSE {
                println!("is_user_triggered_collection");
            }
            return true;
        }
        if self.next_gc_full_heap || self.collection_attempt > 1 {
            if VERBOSE {
                println!("next_gc_full_heap: {}", self.next_gc_full_heap);
                println!("collection_attempt: {}", self.collection_attempt);
            }
            return true;
        }
        if self.virtual_memory_exhausted() {
            if VERBOSE {
                println!("virtual_memory_exhausted");
            }
            return true;
        }
        return false;
    }

    fn virtual_memory_exhausted(&self) -> bool {
        (self.get_collection_reserve() as f32 * WORST_CASE_COPY_EXPANSION) as usize >= self.get_mature_physical_pages_avail()
    }

    fn get_mature_physical_pages_avail(&self) -> usize {
        self.tospace().available_physical_pages() >> 1
    }

    pub fn nursery_space(&self) -> &'static CopySpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.nursery_space
    }

    pub fn tospace(&self) -> &'static CopySpace {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.hi {
            &unsync.copyspace1
        } else {
            &unsync.copyspace0
        }
    }

    pub fn fromspace(&self) -> &'static CopySpace {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.hi {
            &unsync.copyspace0
        } else {
            &unsync.copyspace1
        }
    }

    pub fn get_sstrace(&self) -> &Trace {
        &self.ss_trace
    }

    pub fn get_los(&self) -> &'static LargeObjectSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.los
    }
}

impl ::std::ops::Deref for SemiSpace {
    type Target = SemiSpaceUnsync;
    fn deref(&self) -> &SemiSpaceUnsync {
        unsafe { &*self.unsync.get() }
    }
}

impl ::std::ops::DerefMut for SemiSpace {
    fn deref_mut(&mut self) -> &mut SemiSpaceUnsync {
        unsafe { &mut *self.unsync.get() }
    }
}