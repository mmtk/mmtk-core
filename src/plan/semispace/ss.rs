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
use ::plan::Phase;
use ::plan::trace::Trace;
use ::util::ObjectReference;
use ::util::alloc::allocator::determine_collection_attempts;

use ::util::heap::VMRequest;

use libc::c_void;
use std::cell::UnsafeCell;
use std::sync::atomic::{self, AtomicBool, AtomicUsize, Ordering};

use ::vm::{Scanning, VMScanning};
use std::thread;
use util::conversions::bytes_to_pages;
use plan::plan::create_vm_space;
use plan::plan::EMERGENCY_COLLECTION;

pub type SelectedPlan = SemiSpace;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

lazy_static! {
    pub static ref PLAN: SemiSpace = SemiSpace::new();
}

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
                hi: true,
                vm_space: create_vm_space(),
                copyspace0: CopySpace::new("copyspace0", false, true,
                                           VMRequest::RequestFraction {
                                               frac: 1.0/3.0,
                                               top: false,
                                           }),
                copyspace1: CopySpace::new("copyspace1", true, true,
                                           VMRequest::RequestFraction {
                                               frac: 1.0/3.0,
                                               top: false,
                                           }),
                versatile_space: ImmortalSpace::new("versatile_space", true,
                                                    VMRequest::RequestFraction {
                                                        frac: 1.0/3.0,
                                                        top:  false,
                                                    }),
                total_pages: 0,
                collection_attempt: 0,
            }),
            ss_trace: Trace::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages(heap_size);
        unsync.vm_space.init();
        unsync.copyspace0.init();
        unsync.copyspace1.init();
        unsync.versatile_space.init();

        if !cfg!(feature = "jikesrvm") {
            thread::spawn(|| {
                ::plan::plan::CONTROL_COLLECTOR_CONTEXT.run(0)
            });
        }
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        Box::into_raw(Box::new(SSMutator::new(thread_id, self.tospace(),
                                              &unsync.versatile_space))) as *mut c_void
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        let unsync = unsafe { &*self.unsync.get() };

        if self.tospace().in_space(object) || self.fromspace().in_space(object) {
            return false;
        }

        if unsync.versatile_space.in_space(object) {
            return true;
        }

        // this preserves correctness over efficiency
        false
    }

    unsafe fn collection_phase(&self, thread_id: usize, phase: &Phase) {
        let unsync = &mut *self.unsync.get();

        match phase {
            &Phase::SetCollectionKind => {
                let unsync = unsafe { &mut *self.unsync.get() };
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
                plan::STACKS_PREPARED.store(true, atomic::Ordering::Relaxed);
            }
            &Phase::Prepare => {
                unsync.hi = !unsync.hi; // flip the semi-spaces
                // prepare each of the collected regions
                unsync.copyspace0.prepare(unsync.hi);
                unsync.copyspace1.prepare(!unsync.hi);
                unsync.versatile_space.prepare();
            }
            &Phase::StackRoots => {
                VMScanning::notify_initial_thread_scan_complete(false, thread_id);
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
            }
            &Phase::Complete => {
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
        self.tospace().reserved_pages() + unsync.versatile_space.reserved_pages()
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
}