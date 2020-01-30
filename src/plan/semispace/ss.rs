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
use ::util::sanity::sanity_checker::SanityChecker;
use ::util::sanity::memory_scan;
use ::util::heap::layout::Mmapper as IMmapper;
use ::util::Address;
use ::util::heap::PageResource;
use ::util::heap::VMRequest;
use ::util::OpaquePointer;

use ::util::constants::LOG_BYTES_IN_PAGE;

use libc::{c_void, memset};
use std::cell::UnsafeCell;
use std::sync::atomic::{self, AtomicBool, AtomicUsize, Ordering};

use ::vm::{Scanning, VMScanning};
use std::thread;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;
use plan::plan::EMERGENCY_COLLECTION;
use util::heap::layout::heap_layout::VMMap;
use util::heap::layout::heap_layout::Mmapper;
use util::options::{Options, UnsafeOptionsWrapper};
use std::sync::Arc;

pub type SelectedPlan = SemiSpace;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

pub struct SemiSpace {
    pub unsync: UnsafeCell<SemiSpaceUnsync>,
    pub ss_trace: Trace,
}

pub struct SemiSpaceUnsync {
    pub hi: bool,
    pub vm_space: ImmortalSpace,
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    pub versatile_space: ImmortalSpace,
    pub los: LargeObjectSpace,
    pub mmapper: &'static Mmapper,
    pub options: Arc<UnsafeOptionsWrapper>,
    // FIXME: This should be inside HeapGrowthManager
    total_pages: usize,

    collection_attempt: usize,
}

unsafe impl Sync for SemiSpace {}

impl Plan for SemiSpace {
    type MutatorT = SSMutator;
    type TraceLocalT = SSTraceLocal;
    type CollectorT = SSCollector;

    fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper, options: Arc<UnsafeOptionsWrapper>) -> Self {
        SemiSpace {
            unsync: UnsafeCell::new(SemiSpaceUnsync {
                hi: false,
                vm_space: create_vm_space(vm_map, mmapper),
                copyspace0: CopySpace::new("copyspace0", false, true,
                                           VMRequest::discontiguous(), vm_map, mmapper),
                copyspace1: CopySpace::new("copyspace1", true, true,
                                           VMRequest::discontiguous(), vm_map, mmapper),
                versatile_space: ImmortalSpace::new("versatile_space", true,
                                                    VMRequest::discontiguous(), vm_map, mmapper),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous(), vm_map, mmapper),
                mmapper,
                options,
                total_pages: 0,
                collection_attempt: 0,
            }),
            ss_trace: Trace::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        vm_map.finalize_static_space_map();
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        unsync.vm_space.init(vm_map);
        unsync.copyspace0.init(vm_map);
        unsync.copyspace1.init(vm_map);
        unsync.versatile_space.init(vm_map);
        unsync.los.init(vm_map);

        // These VMs require that the controller thread is started by the VM itself.
        // (Usually because it calls into VM code that accesses the TLS.)
        if !(cfg!(feature = "jikesrvm") || cfg!(feature = "openjdk")) {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(OpaquePointer::UNINITIALIZED)
            });
        }
    }

    fn mmapper(&self) -> &'static Mmapper {
        let unsync = unsafe { &*self.unsync.get() };
        unsync.mmapper
    }

    fn options(&self) -> &Options {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.options
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        Box::into_raw(Box::new(SSMutator::new(tls, self))) as *mut c_void
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

    unsafe fn collection_phase(&self, tls: OpaquePointer, phase: &Phase) {
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
                    self.force_full_heap_collection();
                }
            }
            &Phase::Initiate => {
                plan::set_gc_status(plan::GcStatus::GcPrepare);
            }
            &Phase::PrepareStacks => {
                plan::STACKS_PREPARED.store(true, atomic::Ordering::SeqCst);
            }
            &Phase::Prepare => {
                if cfg!(feature = "sanity") {
                    println!("Pre GC sanity check");
                    SanityChecker::new(tls, &self).check();
                }
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                if cfg!(feature = "sanity") {
                    self.fromspace().unprotect();
                }
                unsync.hi = !unsync.hi; // flip the semi-spaces
                // prepare each of the collected regions
                unsync.copyspace0.prepare(unsync.hi);
                unsync.copyspace1.prepare(!unsync.hi);
                unsync.versatile_space.prepare();
                unsync.vm_space.prepare();
                unsync.los.prepare(true);
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
                if cfg!(feature = "sanity") {
                    if self.fromspace().common().contiguous {
                        let fromspace_start = self.fromspace().common().start;
                        let fromspace_commited = self.fromspace().common().pr.as_ref().unwrap().common().committed.load(Ordering::Relaxed);
                        let commited_bytes = fromspace_commited * (1 << LOG_BYTES_IN_PAGE);
                        println!("Destroying fromspace {}~{}", fromspace_start, fromspace_start + commited_bytes);
                        memset(fromspace_start.as_usize() as *mut c_void, 0xFF, commited_bytes);
                    } else {
                        println!("Fromspace is discontiguous, not destroying")
                    }
                }
                // release the collected region
                if unsync.hi {
                    unsafe { unsync.copyspace0.release() };
                } else {
                    unsafe { unsync.copyspace1.release() };
                }
                unsync.versatile_space.release();
                unsync.vm_space.release();
                unsync.los.release(true);
            }
            &Phase::Complete => {
                if cfg!(feature = "sanity") {
                    println!("Post GC sanity check");
                    SanityChecker::new(tls, &self).check();
                    println!("Post GC memory scan");
                    memory_scan::scan_region(&self);
                    println!("Finished one GC");
                }
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                if cfg!(feature = "sanity") {
                    self.fromspace().protect();
                }
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
        let unsync = unsafe { &*self.unsync.get() };
        if unsync.vm_space.in_space(object) {
            return unsync.vm_space.is_movable();
        }
        if unsync.copyspace0.in_space(object) {
            return unsync.copyspace0.is_movable();
        }
        if unsync.copyspace1.in_space(object) {
            return unsync.copyspace1.is_movable();
        }
        if unsync.versatile_space.in_space(object) {
            return unsync.versatile_space.is_movable();
        }
        if unsync.los.in_space(object) {
            return unsync.los.is_movable();
        }
        return true;
    }

    fn is_mapped_address(&self, address: Address) -> bool {
        let unsync = unsafe { &*self.unsync.get() };
        if unsafe{
            unsync.vm_space.in_space(address.to_object_reference())  ||
            unsync.versatile_space.in_space(address.to_object_reference()) ||
            unsync.copyspace0.in_space(address.to_object_reference()) ||
            unsync.copyspace1.in_space(address.to_object_reference()) ||
            unsync.los.in_space(address.to_object_reference())
        } {
            return unsync.mmapper.address_is_mapped(address);
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

    pub fn get_versatile_space(&self) -> &'static ImmortalSpace {
        let unsync = unsafe { &*self.unsync.get() };
        &unsync.versatile_space
    }

    pub fn get_los(&self) -> &'static LargeObjectSpace {
        let unsync = unsafe { &*self.unsync.get() };

        &unsync.los
    }
}