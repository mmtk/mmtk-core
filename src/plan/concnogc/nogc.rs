use ::policy::space::Space;
use ::policy::immortalspace::ImmortalSpace;
use ::plan::{Plan, Phase};
use ::util::ObjectReference;
use ::util::heap::VMRequest;
use ::util::heap::layout::heap_layout::MMAPPER;
use ::util::heap::layout::Mmapper;
use ::util::Address;
use ::plan::{phase, plan};
use ::vm::*;
use plan::plan::EMERGENCY_COLLECTION;
use ::std::sync::atomic::{AtomicUsize, Ordering};
use ::std::sync::Mutex;
use ::util::heap::PageResource;

use std::cell::UnsafeCell;
use std::thread;
use libc::c_void;

lazy_static! {
    pub static ref PLAN: NoGC = NoGC::new();

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
        (phase::Schedule::Complex, plan::FINISH_PHASE.clone()),
    ], 0);

    pub static ref GC_COUNT: AtomicUsize = AtomicUsize::new(1);

    pub static ref HEAD: Mutex<ObjectReference> = Mutex::new(ObjectReference::null());
}

use super::NoGCTraceLocal;
use super::NoGCMutator;
use super::NoGCCollector;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;
use ::plan::trace::Trace;
use ::util::alloc::allocator::determine_collection_attempts;
use util::queue::SharedQueue;

pub type SelectedPlan = NoGC;

pub struct NoGC {
    pub unsync: UnsafeCell<NoGCUnsync>,
    pub trace: Trace,
    pub modbuf_pool: SharedQueue<ObjectReference>,
}

unsafe impl Sync for NoGC {}

pub struct NoGCUnsync {
    vm_space: ImmortalSpace,
    pub space: ImmortalSpace,
    pub total_pages: usize,
    pub log_state: u16,
    pub mark_state: u16,
    pub collection_attempt: usize,
    pub new_barrier_active: bool,
}

impl Plan for NoGC {
    type MutatorT = NoGCMutator;
    type TraceLocalT = NoGCTraceLocal;
    type CollectorT = NoGCCollector;

    fn new() -> Self {
        NoGC {
            unsync: UnsafeCell::new(NoGCUnsync {
                vm_space: create_vm_space(),
                space: ImmortalSpace::new("nogc", true, VMRequest::fraction(0.9)),
                total_pages: 0,
                log_state: 1,
                mark_state: 0,
                collection_attempt: 0,
                new_barrier_active: false,
            }),
            trace: Trace::new(),
            modbuf_pool: SharedQueue::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        ::util::heap::layout::heap_layout::VM_MAP.finalize_static_space_map();
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        // FIXME correctly initialize spaces based on options
        unsync.vm_space.init();
        unsync.space.init();

        // These VMs require that the controller thread is started by the VM itself.
        // (Usually because it calls into VM code that accesses the TLS.)
        if !(cfg!(feature = "jikesrvm") || cfg!(feature = "openjdk")) {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(0 as *mut c_void)
            });
        }
    }

    fn bind_mutator(&self, tls: *mut c_void) -> *mut c_void {
        // let unsync = unsafe { &*self.unsync.get() };
        Box::into_raw(Box::new(NoGCMutator::new(tls, &self.get_space()))) as *mut c_void
    }

    fn will_never_move(&self, _object: ObjectReference) -> bool {
        true
    }

    unsafe fn collection_phase(&self, tls: *mut c_void, phase: &Phase) {
        if super::VERBOSE {
            println!("Global {:?}", phase);
        }

        let unsync = &mut *self.unsync.get();

        match phase {
            &Phase::SetCollectionKind => {
                // if super::VERBOSE {
                    println!("GC #{}", GC_COUNT.load(Ordering::Relaxed));
                // }

                let unsync = &mut *self.unsync.get();
                unsync.collection_attempt = if <SelectedPlan as Plan>::is_user_triggered_collection() {
                    1 } else { determine_collection_attempts() };

                let emergency_collection = !<SelectedPlan as Plan>::is_internal_triggered_collection()
                    && self.last_collection_was_exhaustive() && unsync.collection_attempt > 1;
                EMERGENCY_COLLECTION.store(emergency_collection, Ordering::Relaxed);

                if emergency_collection {
                    self.force_full_heap_collection();
                }
            }
            &Phase::Initiate => {
                plan::set_gc_status(plan::GcStatus::GcPrepare);
            }
            &Phase::PrepareStacks => {
                plan::STACKS_PREPARED.store(true, ::std::sync::atomic::Ordering::SeqCst);
            }
            &Phase::Prepare => {
                debug_assert!(self.trace.values.is_empty());
                debug_assert!(self.trace.root_locations.is_empty());
                unsync.mark_state += 1;
                while unsync.mark_state == 0 {
                    unsync.mark_state += 1;
                }
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
            &Phase::Release => {}
            &Phase::Complete => {
                debug_assert!(self.modbuf_pool.is_empty());
                debug_assert!(self.trace.values.is_empty());
                debug_assert!(self.trace.root_locations.is_empty());
                self.clear_dead_objects();
                plan::set_gc_status(plan::GcStatus::NotInGC);
                GC_COUNT.fetch_add(1, Ordering::Relaxed);
            }
            &Phase::SetBarrierActive => {
                unsync.new_barrier_active = true;
            }
            &Phase::ClearBarrierActive => {
                unsync.new_barrier_active = false;
            }
            _ => {
                panic!("Global phase not handled!")
            }
        }
    }

    fn get_total_pages(&self) -> usize {
        self.total_pages
    }

    fn get_pages_used(&self) -> usize {
        self.space.reserved_pages()
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.space.in_space(object) {
            return true;
        }
        if unsync.vm_space.in_space(object) {
            return true;
        }
        return false;
    }

    fn is_bad_ref(&self, _object: ObjectReference) -> bool {
        false
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsafe {
            unsync.space.in_space(address.to_object_reference()) ||
            unsync.vm_space.in_space(address.to_object_reference())
        } {
            return MMAPPER.address_is_mapped(address);
        } else {
            return false;
        }
    }

    fn is_movable(&self, _object: ObjectReference) -> bool {
        return false;
    }

    fn collection_required<PR: PageResource>(&self, space_full: bool, _space: &'static PR::Space) -> bool {
        let stress_force_gc = self.stress_test_gc_required();
        trace!("self.get_pages_reserved()={}, self.get_total_pages()={}",
               self.get_pages_reserved(), self.get_total_pages());
        let heap_full = self.get_pages_reserved() > self.get_total_pages();

        space_full || stress_force_gc || heap_full
    }

    fn concurrent_collection_required(&self) -> bool {
        if !::plan::phase::concurrent_phase_active() {
            return true;
        }
        false
    }
}

impl NoGC {
    pub fn get_trace(&self) -> &Trace {
        &self.trace
    }

    pub fn get_space(&self) -> &'static ImmortalSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.space
    }

    pub fn clear_dead_objects(&self) {
        let mut head = HEAD.lock().unwrap();

        while !head.is_null() && !is_marked(*head) {
            *head = get_next(*head);
        }

        let mut parent = *head;
        while !parent.is_null() {
            let next = get_next(parent);
            if !next.is_null() && !is_marked(next) {
                set_next(parent, get_next(next));
                // Zero
                let mut start = VMObjectModel::object_start_ref(next);
                let end = VMObjectModel::get_object_end_address(next);
                while start < end {
                    unsafe { start.store::<usize>(0xdeadbeaf) }
                    start = start + 4usize;
                }
            } else {
                parent = next;
            }
        }
    }
}

impl ::std::ops::Deref for NoGC {
    type Target = NoGCUnsync;
    fn deref(&self) -> &NoGCUnsync {
        unsafe { &*self.unsync.get() }
    }
}

impl ::std::ops::DerefMut for NoGC {
    fn deref_mut(&mut self) -> &mut NoGCUnsync {
        unsafe { &mut *self.unsync.get() }
    }
}

pub fn set_next(object: ObjectReference, next: ObjectReference) {
    let next_address_slot = object.to_address() + (VMObjectModel::GC_HEADER_OFFSET() + 4isize);
    unsafe { next_address_slot.store::<ObjectReference>(next) }
}

pub fn get_next(object: ObjectReference) -> ObjectReference {
    let next_address_slot = object.to_address() + (VMObjectModel::GC_HEADER_OFFSET() + 4isize);
    unsafe { next_address_slot.load::<ObjectReference>() }
}

pub fn is_marked(object: ObjectReference) -> bool {
    let mark_slot = object.to_address() + (VMObjectModel::GC_HEADER_OFFSET() + 2isize);
    (unsafe { mark_slot.load::<u16>() }) == PLAN.mark_state
}