use ::policy::space::Space;

use super::SSMutator;
use super::SSTraceLocal;
use super::SSCollector;
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
use libc::{c_void};
use std::cell::UnsafeCell;
use std::sync::atomic::{self, Ordering};
use ::vm::*;
use std::thread;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;
use plan::plan::EMERGENCY_COLLECTION;
use util::queue::SharedQueue;
use super::VERBOSE;

pub type SelectedPlan = SemiSpace;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

lazy_static! {
    pub static ref PLAN: SemiSpace = SemiSpace::new();

    pub static ref PREEMPT_CONCURRENT_CLOSURE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::FlushMutator),
        (phase::Schedule::Collector, phase::Phase::Closure),
    ], 0);

    pub static ref CONCURRENT_CLOSURE: phase::Phase = phase::Phase::Complex(vec![
      (phase::Schedule::Global,     phase::Phase::SetBarrierActive),
      (phase::Schedule::Mutator,    phase::Phase::SetBarrierActive),
      (phase::Schedule::Collector,  phase::Phase::FlushCollector),
      (phase::Schedule::Concurrent, phase::Phase::Concurrent(
        box (phase::Schedule::Complex, PREEMPT_CONCURRENT_CLOSURE.clone())
      )),
      (phase::Schedule::Global,     phase::Phase::ClearBarrierActive),
      (phase::Schedule::Mutator,    phase::Phase::ClearBarrierActive),
      (phase::Schedule::Mutator,  phase::Phase::FinalClosure),
      (phase::Schedule::Collector,  phase::Phase::FinalClosure),
    ], 0);

    pub static ref ROOT_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::Prepare),
        (phase::Schedule::Global, phase::Phase::Prepare),
        (phase::Schedule::Collector, phase::Phase::Prepare),
        (phase::Schedule::Complex, plan::PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global, phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global, phase::Phase::Roots),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Complex, CONCURRENT_CLOSURE.clone()),
    ], 0);

    pub static ref REF_TYPE_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Complex, CONCURRENT_CLOSURE.clone()),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Complex, CONCURRENT_CLOSURE.clone()),
        (phase::Schedule::Placeholder, phase::Phase::WeakTrackRefs),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs)
    ], 0);

    pub static ref COLLECTION: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Complex, plan::INIT_PHASE.clone()),
        (phase::Schedule::Complex, ROOT_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, REF_TYPE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, plan::COMPLETE_CLOSURE_PHASE.clone()),
        super::validate::schedule_validation_phase(),
        (phase::Schedule::Complex, plan::FINISH_PHASE.clone()),
    ], 0);
}

pub struct SemiSpace {
    pub unsync: UnsafeCell<SemiSpaceUnsync>,
    pub ss_trace: Trace,
    pub modbuf_pool: SharedQueue<ObjectReference>,
}

pub struct SemiSpaceUnsync {
    pub hi: bool,
    pub vm_space: ImmortalSpace,
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    pub versatile_space: ImmortalSpace,
    pub los: LargeObjectSpace,
    total_pages: usize,
    collection_attempt: usize,
    pub log_state: usize,
    pub new_barrier_active: bool,
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
                copyspace0: CopySpace::new("copyspace0", false, true, VMRequest::discontiguous()),
                copyspace1: CopySpace::new("copyspace1", true, true, VMRequest::discontiguous()),
                versatile_space: ImmortalSpace::new("versatile_space", true, VMRequest::discontiguous()),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous()),
                total_pages: 0,
                collection_attempt: 0,
                log_state: 1,
                new_barrier_active: false,
            }),
            ss_trace: Trace::new(),
            modbuf_pool: SharedQueue::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        ::util::heap::layout::heap_layout::VM_MAP.finalize_static_space_map();
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        unsync.vm_space.init();
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
        Box::into_raw(Box::new(SSMutator::new(tls, self.tospace(), &unsync.versatile_space, &unsync.los))) as *mut c_void
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };

        if self.tospace().in_space(object) || self.fromspace().in_space(object) {
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
        if self.tospace().in_space(object) {
            return true;
        }
        if unsync.los.in_space(object) {
            return true;
        }
        return false;
    }

    fn collection_required<PR: PageResource>(&self, space_full: bool, space: &'static PR::Space) -> bool where Self: Sized {
        let stress_force_gc = self.stress_test_gc_required();
        trace!("self.get_pages_reserved()={}, self.get_total_pages()={}",
               self.get_pages_reserved(), self.get_total_pages());
        {
            let used = self.tospace().reserved_pages() as f32;
            let total = (self.get_total_pages() >> 1) as f32;
            if used / total > 0.7f32 {
                return true;
            }
        }
        
        let heap_full = self.get_pages_reserved() > self.get_total_pages();
        space_full || stress_force_gc || heap_full
    }

    fn concurrent_collection_required(&self) -> bool {
        if !::plan::phase::concurrent_phase_active() {
            // return self.get_pages_used() as f32 / self.get_total_pages() as f32 > 0.3f32;
            let used = self.tospace().reserved_pages() as f32;
            let total = (self.get_total_pages() >> 1) as f32;
            if used / total > 0.3f32 {
                return true;
            }
        }
        false
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

                if VERBOSE {
                    unsync.copyspace0.print_vm_map();
                    unsync.copyspace1.print_vm_map();
                    unsync.versatile_space.print_vm_map();
                    unsync.los.print_vm_map();
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
                unsync.hi = !unsync.hi; // flip the semi-spaces
                // prepare each of the collected regions
                unsync.copyspace0.prepare(unsync.hi);
                unsync.copyspace1.prepare(!unsync.hi);
                unsync.versatile_space.prepare();
                unsync.vm_space.prepare();
                unsync.los.prepare(true);
                unsync.log_state += 1;
                while unsync.log_state == 0 {
                    unsync.log_state += 1;
                }
            }
            &Phase::StackRoots => {
                VMScanning::notify_initial_thread_scan_complete(false, tls);
                plan::set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Roots => {
                VMScanning::reset_thread_counter();
                plan::set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Closure => {}
            &Phase::Release => {
                // release the collected region
                if unsync.hi {
                    unsync.copyspace0.release();
                } else {
                    unsync.copyspace1.release();
                }
                unsync.versatile_space.release();
                unsync.vm_space.release();
                unsync.los.release(true);
            }
            &Phase::Complete => {
                if VERBOSE {
                    unsync.copyspace0.print_vm_map();
                    unsync.copyspace1.print_vm_map();
                    unsync.versatile_space.print_vm_map();
                    unsync.los.print_vm_map();
                    unsync.vm_space.print_vm_map();
                }
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                plan::set_gc_status(plan::GcStatus::NotInGC);
            }
            &Phase::SetBarrierActive => {
                unsync.new_barrier_active = true;
            }
            &Phase::ClearBarrierActive => {
                unsync.new_barrier_active = false;
            }
            &Phase::ValidatePrepare => {
                super::validate::prepare();
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                if VERBOSE {
                    unsync.copyspace0.print_vm_map();
                    unsync.copyspace1.print_vm_map();
                    unsync.versatile_space.print_vm_map();
                    unsync.los.print_vm_map();
                    unsync.vm_space.print_vm_map();
                }
                // unsync.remset_pool.clear();
            }
            &Phase::ValidateRelease => {
                super::validate::release();
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
        self.tospace().reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        let unsync = unsafe{&*self.unsync.get()};
        self.tospace().reserved_pages() + unsync.versatile_space.reserved_pages() + unsync.los.reserved_pages()
    }

    fn is_bad_ref(&self, object: ObjectReference) -> bool {
        self.fromspace().in_space(object)
    }

    fn is_movable(&self, object: ObjectReference) -> bool {
        if self.vm_space.in_space(object) {
            return self.vm_space.is_movable();
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

pub fn log(object: ObjectReference) -> bool {
    let log_slot = object.to_address() + (VMObjectModel::GC_HEADER_OFFSET());
    if (unsafe { log_slot.load::<usize>() }) == PLAN.log_state {
        false
    } else {
        unsafe { log_slot.store::<usize>(PLAN.log_state) };
        true
    }
}