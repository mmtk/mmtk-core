use ::policy::space::Space;
use super::G1Mutator;
use super::G1TraceLocal;
use super::G1Collector;
use ::plan::plan;
use ::plan::phase;
use ::plan::Plan;
use ::plan::trace::Trace;
use ::plan::Allocator;
use ::policy::immortalspace::ImmortalSpace;
use ::plan::Phase;
use ::util::ObjectReference;
use ::util::alloc::allocator::determine_collection_attempts;
use ::util::heap::layout::heap_layout::MMAPPER;
use ::util::heap::layout::Mmapper;
use ::util::Address;
use ::util::heap::PageResource;
use ::util::heap::VMRequest;
use libc::c_void;
use std::cell::UnsafeCell;
use std::sync::atomic::{self, Ordering};
use ::vm::{Scanning, VMScanning};
use std::thread;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;
use plan::plan::EMERGENCY_COLLECTION;
use policy::regionspace::*;
use super::DEBUG;
use policy::largeobjectspace::LargeObjectSpace;


pub type SelectedPlan = G1;

pub const ALLOC_RS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

lazy_static! {
    pub static ref PLAN: G1 = G1::new();
    pub static ref EVACUATE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator,   phase::Phase::EvacuatePrepare),
        (phase::Schedule::Global,    phase::Phase::EvacuatePrepare),
        (phase::Schedule::Collector, phase::Phase::EvacuatePrepare),
        // Roots
        (phase::Schedule::Complex,   plan::PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global,    phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global,    phase::Phase::Roots),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        // Refs
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs),

        (phase::Schedule::Mutator,   phase::Phase::EvacuateRelease),
        (phase::Schedule::Global,    phase::Phase::EvacuateRelease),
        (phase::Schedule::Collector, phase::Phase::EvacuateRelease),
    ], 0);
    pub static ref COLLECTION: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Complex, plan::INIT_PHASE.clone()),
        (phase::Schedule::Complex, plan::ROOT_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, plan::REF_TYPE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, plan::COMPLETE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Global,  phase::Phase::CollectionSetSelection),
        (phase::Schedule::Complex, EVACUATE_PHASE.clone()),
        (phase::Schedule::Complex, plan::FINISH_PHASE.clone()),
    ], 0);
}

pub struct G1 {
    pub unsync: UnsafeCell<G1Unsync>,
    pub mark_trace: Trace,
    pub evacuate_trace: Trace,
}

pub struct G1Unsync {
    pub hi: bool,
    pub vm_space: ImmortalSpace,
    pub region_space: RegionSpace,
    pub los: LargeObjectSpace,
    pub versatile_space: ImmortalSpace,
    total_pages: usize,
    collection_attempt: usize,
}

unsafe impl Sync for G1 {}

impl ::std::ops::Deref for G1 {
    type Target = G1Unsync;
    fn deref(&self) -> &G1Unsync {
        unsafe { &*self.unsync.get() }
    }
}

impl ::std::ops::DerefMut for G1 {
    fn deref_mut(&mut self) -> &mut G1Unsync {
        unsafe { &mut *self.unsync.get() }
    }
}

impl Plan for G1 {
    type MutatorT = G1Mutator;
    type TraceLocalT = G1TraceLocal;
    type CollectorT = G1Collector;

    fn new() -> Self {
        G1 {
            unsync: UnsafeCell::new(G1Unsync {
                hi: false,
                vm_space: create_vm_space(),
                region_space: RegionSpace::new("region_space", VMRequest::discontiguous()),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous()),
                versatile_space: ImmortalSpace::new("versatile_space", true, VMRequest::discontiguous()),
                total_pages: 0,
                collection_attempt: 0,
            }),
            mark_trace: Trace::new(),
            evacuate_trace: Trace::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        ::util::heap::layout::heap_layout::VM_MAP.finalize_static_space_map();
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        unsync.vm_space.init();
        unsync.region_space.init();
        unsync.los.init();
        unsync.versatile_space.init();

        if !cfg!(feature = "jikesrvm") {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(0 as *mut c_void)
            });
        }
    }

    fn bind_mutator(&self, tls: *mut c_void) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        Box::into_raw(Box::new(G1Mutator::new(tls, &unsync.region_space, &unsync.los, &unsync.versatile_space))) as *mut c_void
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        if self.region_space.in_space(object) {
            false
        } else if self.versatile_space.in_space(object) {
            true
        } else if self.los.in_space(object) {
            true
        } else if self.vm_space.in_space(object) {
            true
        } else {
            unreachable!()
        }
    }

    fn is_valid_ref(&self, object: ObjectReference) -> bool {
        if self.region_space.in_space(object) {
            true
        } else if self.versatile_space.in_space(object) {
            true
        } else if self.los.in_space(object) {
            true
        } else if self.vm_space.in_space(object) {
            true
        } else {
            false
        }
    }

    unsafe fn collection_phase(&self, tls: *mut c_void, phase: &Phase) {
        if DEBUG {
            println!("Global {:?}", phase);
        }
        let unsync = &mut *self.unsync.get();

        match phase {
            &Phase::SetCollectionKind => {
                unsync.collection_attempt = if <SelectedPlan as Plan>::is_user_triggered_collection() {
                    1
                } else {
                    determine_collection_attempts()
                };
                let emergency_collection = !<SelectedPlan as Plan>::is_internal_triggered_collection()
                    && self.last_collection_was_exhaustive()
                    && unsync.collection_attempt > 1;
                EMERGENCY_COLLECTION.store(emergency_collection, Ordering::Relaxed);
                if emergency_collection {
                    self.force_full_heap_collection();
                }
            },
            &Phase::Initiate => {
                plan::set_gc_status(plan::GcStatus::GcPrepare);
            },
            &Phase::PrepareStacks => {
                plan::STACKS_PREPARED.store(true, atomic::Ordering::SeqCst);
            },
            &Phase::Prepare => {
                debug_assert!(self.mark_trace.values.is_empty());
                debug_assert!(self.mark_trace.root_locations.is_empty());
                // prepare each of the collected regions
                unsync.region_space.prepare();
                unsync.los.prepare(true);
                unsync.versatile_space.prepare();
                unsync.vm_space.prepare();
                self.print_vm_map();
            },
            &Phase::StackRoots => {
                VMScanning::notify_initial_thread_scan_complete(false, tls);
                plan::set_gc_status(plan::GcStatus::GcProper);
            },
            &Phase::Roots => {
                VMScanning::reset_thread_counter();
                plan::set_gc_status(plan::GcStatus::GcProper);
            },
            &Phase::Closure => {},
            &Phase::Release => {
                // unsync.region_space.release();
                unsync.versatile_space.release();
                unsync.los.release(true);
                unsync.vm_space.release();
            },
            &Phase::CollectionSetSelection => {
                self.region_space.compute_collection_set(self.get_total_pages() - self.get_pages_used());
            },
            &Phase::EvacuatePrepare => {
                debug_assert!(self.evacuate_trace.values.is_empty());
                debug_assert!(self.evacuate_trace.root_locations.is_empty());
                // prepare each of the collected regions
                unsync.region_space.prepare();
                unsync.versatile_space.prepare();
                unsync.los.prepare(true);
                unsync.vm_space.prepare();
            },
            &Phase::EvacuateClosure => {},
            &Phase::EvacuateRelease => {
                unsync.region_space.release();
                unsync.los.release(true);
                unsync.versatile_space.release();
                unsync.vm_space.release();
            },
            &Phase::Complete => {
                debug_assert!(self.mark_trace.values.is_empty());
                debug_assert!(self.mark_trace.root_locations.is_empty());
                debug_assert!(self.evacuate_trace.values.is_empty());
                debug_assert!(self.evacuate_trace.root_locations.is_empty());
                plan::set_gc_status(plan::GcStatus::NotInGC);
                self.print_vm_map();
            },

            _ => panic!("Global phase not handled!"),
        }
    }

    #[inline]
    fn collection_required<PR: PageResource>(&self, space_full: bool, space: &'static PR::Space) -> bool {
        let total_pages = self.get_total_pages();
        // if self.get_pages_avail() * 10 < total_pages {
        //     return true;
        // }
        let heap_full = self.get_pages_reserved() > total_pages;
        space_full || heap_full
    }

    fn get_total_pages(&self) -> usize {
        self.total_pages
    }

    fn get_collection_reserve(&self) -> usize {
        // println!("{} {}", self.total_pages, self.total_pages / 10);
        self.total_pages / 10
        // self.region_space.reserved_pages()
    }

    fn get_pages_used(&self) -> usize {
        self.region_space.reserved_pages() + self.los.reserved_pages() + self.versatile_space.reserved_pages()
    }

    fn is_bad_ref(&self, object: ObjectReference) -> bool {
        !self.is_valid_ref(object)
    }

    fn is_movable(&self, object: ObjectReference) -> bool {
        if self.vm_space.in_space(object) {
            self.vm_space.is_movable()
        } else if self.region_space.in_space(object) {
            self.region_space.is_movable()
        } else if self.los.in_space(object) {
            self.los.is_movable()
        } else if self.versatile_space.in_space(object) {
            self.versatile_space.is_movable()
        } else {
            unreachable!()
        }
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        let object = unsafe { address.to_object_reference() };
        if self.vm_space.in_space(object)
          || self.versatile_space.in_space(object)
          || self.region_space.in_space(object)
          || self.los.in_space(object) {
            MMAPPER.address_is_mapped(address)
        } else {
            false
        }
    }
}

impl G1 {
    pub fn get_los(&self) -> &'static LargeObjectSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.los
    }

    fn print_vm_map(&self) {
        if super::DEBUG {
            self.region_space.print_vm_map();
            self.los.print_vm_map();
            self.versatile_space.print_vm_map();
            self.vm_space.print_vm_map();
        }
    }
}