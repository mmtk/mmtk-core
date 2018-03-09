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

use ::util::heap::VMRequest;

use libc::c_void;
use std::cell::UnsafeCell;
use std::sync::atomic::{self, AtomicBool};

use ::vm::{Scanning, VMScanning};
use std::thread;
use util::conversions::bytes_to_pages;

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
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    pub versatile_space: ImmortalSpace,

    // FIXME: This should be inside HeapGrowthManager
    total_pages: usize,
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
                copyspace0: CopySpace::new("copyspace0", false, true,
                                           VMRequest::RequestFraction {
                                               frac: 0.3,
                                               top: false,
                                           }),
                copyspace1: CopySpace::new("copyspace1", true, true,
                                           VMRequest::RequestFraction {
                                               frac: 0.3,
                                               top: false,
                                           }),
                versatile_space: ImmortalSpace::new("versatile_space", true,
                                                    VMRequest::RequestFraction {
                                                        frac: 0.3,
                                                        top:  false,
                                                    }),
                total_pages: 0,
            }),
            ss_trace: Trace::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        let unsync = &mut *self.unsync.get();
        unsync.total_pages = bytes_to_pages((0.9 * heap_size as f64) as usize);
        // FIXME correctly initialize spaces based on options
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
        Box::into_raw(Box::new(SSMutator::new(thread_id, self.fromspace(),
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
                // FIXME emergency collection, etc.
            }
            &Phase::Initiate => {
                plan::set_gc_status(plan::GcStatus::GcPrepare);
            }
            &Phase::PrepareStacks => {
                plan::STACKS_PREPARED.store(true, atomic::Ordering::Relaxed);
            }
            &Phase::Prepare => {
                unsync.hi = !unsync.hi;
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
                self.fromspace().release();
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