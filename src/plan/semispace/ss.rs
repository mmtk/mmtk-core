use super::super::plan::default;

use ::policy::space::Space;

use super::SSMutator;
use super::SSTraceLocal;
use super::SSCollector;

use ::plan::controller_collector_context::ControllerCollectorContext;

use ::plan::Plan;
use ::plan::Allocator;
use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::plan::Phase;
use ::plan::trace::Trace;
use ::util::ObjectReference;

use libc::c_void;
use std::cell::UnsafeCell;

pub type SelectedPlan<'a> = SemiSpace<'a>;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

lazy_static! {
    pub static ref PLAN: SemiSpace<'static> = SemiSpace::new();
}

pub struct SemiSpace<'a> {
    pub control_collector_context: ControllerCollectorContext<'a>,
    pub unsync: UnsafeCell<SemiSpaceUnsync>,
}

pub struct SemiSpaceUnsync {
    pub hi: bool,
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    pub ss_trace: Trace,
    pub versatile_space: ImmortalSpace,
}

unsafe impl<'a> Sync for SemiSpace<'a> {}

impl<'a> Plan for SemiSpace<'a> {
    type MutatorT = SSMutator<'a>;
    type TraceLocalT = SSTraceLocal;
    type CollectorT = SSCollector<'a>;

    fn new() -> Self {
        SemiSpace {
            control_collector_context: ControllerCollectorContext::new(),
            unsync: UnsafeCell::new(SemiSpaceUnsync {
                hi: false,
                copyspace0: CopySpace::new(false),
                copyspace1: CopySpace::new(true),
                ss_trace: Trace::new(),
                versatile_space: ImmortalSpace::new(),
            }),
        }
    }

    unsafe fn gc_init(&self, heap_size: usize) {
        let unsync = &mut *self.unsync.get();
        // FIXME
        default::gc_init(&unsync.copyspace0, heap_size / 3);
        unsync.copyspace1.init(heap_size / 3);
        unsync.versatile_space.init(heap_size / 3);
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        let unsync = unsafe { &*self.unsync.get() };
        default::bind_mutator(Self::MutatorT::new(thread_id, self.fromspace(), &unsync.versatile_space))
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

    unsafe fn collection_phase(&self, phase: &Phase) {
        let unsync = &mut *self.unsync.get();

        match phase {
            &Phase::SetCollectionKind => {
                unimplemented!()
            }
            &Phase::Initiate => {
                unimplemented!()
            }
            &Phase::PrepareStacks => {
                unimplemented!()
            }
            &Phase::Prepare => {
                unsync.hi = !unsync.hi;
                unsync.copyspace0.prepare(unsync.hi);
                unsync.copyspace1.prepare(!unsync.hi);
            }
            &Phase::StackRoots => {
                unimplemented!()
            }
            &Phase::Roots => {
                unimplemented!()
            }
            &Phase::Closure => {
                unsync.ss_trace.prepare();
            }
            &Phase::Release => {
                self.fromspace().release();
            }
            &Phase::Complete => {
                unimplemented!()
            }
            _ => {
                panic!("Global phase not handled!")
            }
        }
    }
}

impl<'a> SemiSpace<'a> {
    pub fn tospace(&self) -> &CopySpace {
        let unsync = unsafe { &*self.unsync.get() };

        if unsync.hi {
            &unsync.copyspace1
        } else {
            &unsync.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace {
        let unsync = unsafe { &*self.unsync.get() };

        if unsync.hi {
            &unsync.copyspace0
        } else {
            &unsync.copyspace1
        }
    }
}