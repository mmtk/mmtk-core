use super::super::plan::default;

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

pub type SelectedPlan = SemiSpace;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

lazy_static! {
    pub static ref PLAN: SemiSpace = SemiSpace::new();
}

pub struct SemiSpace {
    pub control_collector_context: ControllerCollectorContext,
    pub unsync: UnsafeCell<SemiSpaceUnsync>,
    pub ss_trace: Trace,
}

pub struct SemiSpaceUnsync {
    pub hi: bool,
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    pub versatile_space: ImmortalSpace,
}

unsafe impl Sync for SemiSpace {}

impl Plan for SemiSpace {
    type MutatorT = SSMutator;
    type TraceLocalT = SSTraceLocal;
    type CollectorT = SSCollector;

    fn new() -> Self {
        SemiSpace {
            control_collector_context: ControllerCollectorContext::new(),
            unsync: UnsafeCell::new(SemiSpaceUnsync {
                hi: false,
                copyspace0: CopySpace::new(false),
                copyspace1: CopySpace::new(true),
                versatile_space: ImmortalSpace::new("versatile_space", true,
                                                    VMRequest::RequestFraction{
                                                        frac: 0.3,
                                                        top:  false,
                                                    }),
            }),
            ss_trace: Trace::new(),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        let unsync = &mut *self.unsync.get();
        // FIXME correctly initialize spaces based on options
        default::gc_init(&mut unsync.copyspace0);
        unsync.copyspace1.init();
        unsync.versatile_space.init();
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        default::bind_mutator(SSMutator::new(thread_id, self.fromspace(), &unsync.versatile_space))
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