use ::policy::space::Space;
use super::G1Mutator;
use super::G1TraceLocal;
use super::G1Collector;
use ::plan::plan;
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
use ::vm::*;
use std::thread;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;
use plan::plan::EMERGENCY_COLLECTION;
use policy::region::*;
use super::VERBOSE;
use policy::largeobjectspace::LargeObjectSpace;
use util::queue::SharedQueue;
use super::predictor::PauseTimePredictor;
use plan::parallel_collector::ParallelCollector;


pub type SelectedPlan = G1;

pub const ALLOC_EDEN: Allocator = Allocator::Default;
pub const ALLOC_SURVIVOR: Allocator = Allocator::G1Survivor;
pub const ALLOC_OLD: Allocator = Allocator::G1Old;

lazy_static! {
    pub static ref PLAN: G1 = G1::new();
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum GCKind {
    Young, Mixed, Full
}

pub struct G1 {
    pub unsync: UnsafeCell<G1Unsync>,
    pub mark_trace: Trace,
    pub evacuate_trace: Trace,
    pub modbuf_pool: SharedQueue<ObjectReference>,
}

pub struct G1Unsync {
    pub hi: bool,
    pub vm_space: ImmortalSpace,
    pub region_space: RegionSpace,
    pub los: LargeObjectSpace,
    pub versatile_space: ImmortalSpace,
    total_pages: usize,
    collection_attempt: usize,
    pub new_barrier_active: bool,
    pub in_gc: bool,
    pub gc_kind: GCKind,
    pub predictor: PauseTimePredictor,
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
                new_barrier_active: false,
                in_gc: false,
                gc_kind: GCKind::Young,
                predictor: PauseTimePredictor::new(),
            }),
            mark_trace: Trace::new(),
            evacuate_trace: Trace::new(),
            modbuf_pool: SharedQueue::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        ::util::heap::layout::heap_layout::VM_MAP.finalize_static_space_map();
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        unsync.vm_space.init();
        unsync.region_space.heap_size = heap_size;
        unsync.region_space.init();
        unsync.los.init();
        unsync.versatile_space.init();

        
        if super::ENABLE_REMEMBERED_SETS {
            super::concurrent_refine::spawn_refine_threads();
        }

        if !cfg!(feature = "jikesrvm") {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(0 as *mut c_void)
            });
        }
    }

    fn bind_mutator(&self, tls: *mut c_void) -> *mut c_void {
        let unsync = unsafe { &mut *self.unsync.get() };
        Box::into_raw(Box::new(G1Mutator::new(tls, &mut unsync.region_space, &unsync.los, &unsync.versatile_space))) as *mut c_void
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
        if VERBOSE {
            println!("Global {:?} {:?}", phase, PLAN.gc_kind);
        }
        let unsync = &mut *self.unsync.get();

        match phase {
            &Phase::SetCollectionKind => {
                unsync.in_gc = true;
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
                unsync.region_space.clear_next_mark_tables();
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
                debug_assert!(self.mark_trace.values.is_empty());
                debug_assert!(self.mark_trace.root_locations.is_empty());
                if !super::ENABLE_REMEMBERED_SETS {
                    unsync.versatile_space.release();
                    unsync.los.release(true);
                    unsync.vm_space.release();
                }
            },
            &Phase::CollectionSetSelection => {
                unsync.predictor.pause_start(
                    VMActivePlan::collector(tls).parallel_worker_count(),
                    PLAN.region_space.nursery_regions(),
                );
                let available_pages = self.get_total_pages() - self.get_pages_used();
                let predictor = self.predictor.get_accumulative_predictor(cardtable::num_dirty_cards());
                if super::ENABLE_GENERATIONAL_GC && self.gc_kind == GCKind::Young {
                    self.region_space.compute_collection_set_for_nursery_gc(available_pages, predictor);
                } else if self.gc_kind == GCKind::Mixed {
                    self.region_space.compute_collection_set_for_mixed_gc(available_pages, predictor);
                } else {
                    self.region_space.compute_collection_set_full_heap(available_pages);
                }
            },
            &Phase::EvacuatePrepare => {
                if super::ENABLE_REMEMBERED_SETS {
                    super::concurrent_refine::disable_concurrent_refinement();
                }
                debug_assert!(self.evacuate_trace.values.is_empty());
                debug_assert!(self.evacuate_trace.root_locations.is_empty());
                // prepare each of the collected regions
                if PLAN.gc_kind != GCKind::Young {
                    unsync.region_space.shift_mark_tables();
                } else {
                    unsync.region_space.clear_next_mark_tables();
                }
                if !super::ENABLE_REMEMBERED_SETS {
                    unsync.region_space.prepare();
                    unsync.versatile_space.prepare();
                    unsync.los.prepare(true);
                    unsync.vm_space.prepare();
                } else {
                    unsync.region_space.reset_alloc_regions();
                }
                
                self.print_vm_map();
            },
            &Phase::RefineCards => {
                // if super::USE_REMEMBERED_SETS {
                //     super::concurrent_refine::disable_concurrent_refinement();
                // }
            }
            &Phase::EvacuateClosure => {},
            &Phase::EvacuateRelease => {
                debug_assert!(self.evacuate_trace.values.is_empty());
                debug_assert!(self.evacuate_trace.root_locations.is_empty());
                unsync.region_space.release();
                if PLAN.gc_kind != GCKind::Young {
                    unsync.los.release(true);
                    unsync.versatile_space.release();
                    unsync.vm_space.release();
                }
            },
            &Phase::Complete => {
                debug_assert!(self.mark_trace.values.is_empty());
                debug_assert!(self.mark_trace.root_locations.is_empty());
                debug_assert!(self.evacuate_trace.values.is_empty());
                debug_assert!(self.evacuate_trace.root_locations.is_empty());
                plan::set_gc_status(plan::GcStatus::NotInGC);
                self.print_vm_map();
                if super::ENABLE_REMEMBERED_SETS {
                    super::concurrent_refine::enable_concurrent_refinement();
                }
                unsync.in_gc = false;
                unsync.predictor.pause_end(self.gc_kind);
            },
            &Phase::SetBarrierActive => {
                unsync.new_barrier_active = true;
            }
            &Phase::ClearBarrierActive => {
                unsync.new_barrier_active = false;
            },
            &Phase::ValidatePrepare => {
                self.print_vm_map();
                super::validate::prepare();
            }
            &Phase::ValidateRelease => {
                super::validate::release();
            }
            _ => panic!("Global phase not handled!"),
        }
    }

    fn force_full_heap_collection(&self) {
        let unsync = unsafe { &mut *self.unsync.get() };
        unsync.gc_kind = GCKind::Full;
    }

    fn collection_required<PR: PageResource>(&self, space_full: bool, _space: &'static PR::Space) -> bool where Self: Sized {
        let stress_force_gc = self.stress_test_gc_required();
        trace!("self.get_pages_reserved()={}, self.get_total_pages()={}",
               self.get_pages_reserved(), self.get_total_pages());
        let heap_full = self.get_pages_reserved() > self.get_total_pages();
        let me = unsafe { &mut *(self as *const _ as usize as *mut Self) };
        if space_full || stress_force_gc || heap_full {
            // Mixed GC
            me.gc_kind = GCKind::Full;
            return true;
        }
        if super::ENABLE_GENERATIONAL_GC && !PLAN.in_gc {
            // if PLAN.region_space.nursery_ratio() > self.predictor.nursery_ratio {
            if self.predictor.within_nursery_budget(cardtable::num_dirty_cards()) {
                me.gc_kind = GCKind::Young;
                return true;
            }
        }
        false
    }

    fn concurrent_collection_required(&self) -> bool {
        if super::ENABLE_CONCURRENT_MARKING && !::plan::phase::concurrent_phase_active() {
            // let used = self.get_pages_used() as f32;
            // let total = self.get_total_pages() as f32;
            // if (used / total) > 0.45f32 {
            if PLAN.region_space.committed_ratio() > 0.45 {
                PLAN.as_mut().gc_kind = GCKind::Mixed;
                return true;
            }
        }
        false
    }

    fn handle_user_collection_request(tls: *mut c_void) {
        if !::util::options::OPTION_MAP.ignore_system_g_c {
            PLAN.as_mut().gc_kind = GCKind::Full;
            ::plan::plan::USER_TRIGGERED_COLLECTION.store(true, Ordering::Relaxed);
            ::plan::plan::CONTROL_COLLECTOR_CONTEXT.request();
            VMCollection::block_for_gc(tls);
        }
    }

    fn get_total_pages(&self) -> usize {
        self.total_pages
    }

    fn get_collection_reserve(&self) -> usize {
        self.region_space.reserved_pages() / 10
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
    pub fn region_space(&self) -> &'static RegionSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.region_space
    }

    pub fn get_los(&self) -> &'static LargeObjectSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.los
    }
    
    pub fn as_mut(&self) -> &mut Self {
        unsafe { &mut *(self as *const _ as usize as *mut _) }
    }

    fn print_vm_map(&self) {
        if super::VERBOSE {
            self.region_space.print_vm_map();
            self.los.print_vm_map();
            self.versatile_space.print_vm_map();
            self.vm_space.print_vm_map();
        }
    }

    pub fn is_mapped_object(&self, object: ObjectReference) -> bool {
        if self.vm_space.in_space(object)
          || self.versatile_space.in_space(object)
          || self.region_space.in_space(object)
          || self.los.in_space(object) {
            MMAPPER.address_is_mapped(VMObjectModel::ref_to_address(object))
        } else {
            false
        }
    }
}