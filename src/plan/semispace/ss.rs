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
use plan::plan::{create_vm_space, CommonPlan};
use util::heap::layout::heap_layout::VMMap;
use util::heap::layout::heap_layout::Mmapper;
use util::options::{Options, UnsafeOptionsWrapper};
use std::sync::Arc;
use util::heap::HeapMeta;
use util::heap::layout::vm_layout_constants::{HEAP_START, HEAP_END};

pub type SelectedPlan = SemiSpace;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

pub struct SemiSpace {
    pub unsync: UnsafeCell<SemiSpaceUnsync>,
    pub ss_trace: Trace,
    pub common: CommonPlan,
}

pub struct SemiSpaceUnsync {
    pub hi: bool,
    pub vm_space: ImmortalSpace,
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    pub versatile_space: ImmortalSpace,
    pub los: LargeObjectSpace,

    // TODO: Check if we really need this. We have collection_attempt in CommonPlan.
    collection_attempt: usize,
}

unsafe impl Sync for SemiSpace {}

impl Plan for SemiSpace {
    type MutatorT = SSMutator;
    type TraceLocalT = SSTraceLocal;
    type CollectorT = SSCollector;

    fn new(vm_map: &'static VMMap, mmapper: &'static Mmapper, options: Arc<UnsafeOptionsWrapper>) -> Self {
        let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

        SemiSpace {
            unsync: UnsafeCell::new(SemiSpaceUnsync {
                hi: false,
                vm_space: create_vm_space(vm_map, mmapper, &mut heap),
                copyspace0: CopySpace::new("copyspace0", false, true,
                                           VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                copyspace1: CopySpace::new("copyspace1", true, true,
                                           VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                versatile_space: ImmortalSpace::new("versatile_space", true,
                                                    VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                los: LargeObjectSpace::new("los", true, VMRequest::discontiguous(), vm_map, mmapper, &mut heap),
                collection_attempt: 0,
            }),
            ss_trace: Trace::new(),
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize, vm_map: &'static VMMap) {
        vm_map.finalize_static_space_map(self.common.heap.get_discontig_start(), self.common.heap.get_discontig_end());

        let unsync = &mut *self.unsync.get();
        self.common.heap.total_pages.store(bytes_to_pages(heap_size), Ordering::Relaxed);
        unsync.vm_space.init(vm_map);
        unsync.copyspace0.init(vm_map);
        unsync.copyspace1.init(vm_map);
        unsync.versatile_space.init(vm_map);
        unsync.los.init(vm_map);
    }

    fn common(&self) -> &CommonPlan {
        &self.common
    }

    fn bind_mutator(&'static self, tls: OpaquePointer) -> *mut c_void {
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
                unsync.collection_attempt = if self.is_user_triggered_collection() {
                    1
                } else {
                    self.determine_collection_attempts()
                };

                let emergency_collection = !self.is_internal_triggered_collection()
                    && self.last_collection_was_exhaustive() && unsync.collection_attempt > 1;
                self.common().emergency_collection.store(emergency_collection, Ordering::Relaxed);

                if emergency_collection {
                    self.force_full_heap_collection();
                }
            }
            &Phase::Initiate => {
                self.common.set_gc_status(plan::GcStatus::GcPrepare);
            }
            &Phase::PrepareStacks => {
                self.common.stacks_prepared.store(true, atomic::Ordering::SeqCst);
            }
            &Phase::Prepare => {
                #[cfg(feature = "sanity")]
                {
                    use ::util::sanity::sanity_checker::SanityChecker;
                    println!("Pre GC sanity check");
                    SanityChecker::new(tls, &self).check();
                }
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                #[cfg(feature = "sanity")]
                {
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
                self.common.set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Roots => {
                VMScanning::reset_thread_counter();
                self.common.set_gc_status(plan::GcStatus::GcProper);
            }
            &Phase::Closure => {}
            &Phase::Release => {
                #[cfg(feature = "sanity")]
                {
                    use ::util::sanity::sanity_checker::SanityChecker;
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
                #[cfg(feature = "sanity")]
                {
                    use ::util::sanity::sanity_checker::SanityChecker;
                    use ::util::sanity::memory_scan;
                    println!("Post GC sanity check");
                    SanityChecker::new(tls, &self).check();
                    println!("Post GC memory scan");
                    memory_scan::scan_region(&self);
                    println!("Finished one GC");
                }
                debug_assert!(self.ss_trace.values.is_empty());
                debug_assert!(self.ss_trace.root_locations.is_empty());
                #[cfg(feature = "sanity")]
                {
                    self.fromspace().protect();
                }

                self.common.set_gc_status(plan::GcStatus::NotInGC);
            }
            _ => {
                panic!("Global phase not handled!")
            }
        }
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
            return self.common.mmapper.address_is_mapped(address);
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