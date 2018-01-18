use super::super::plan::default;

use ::policy::space::Space;

use super::SSMutator;
use super::SSTraceLocal;
use super::SSCollector;
use super::SSConstraints;

use ::plan::controller_collector_context::ControllerCollectorContext;

use ::plan::Plan;
use ::plan::Allocator;
use ::policy::copyspace::CopySpace;
use ::policy::immortalspace::ImmortalSpace;
use ::plan::Phase;
use ::plan::trace::Trace;
use ::util::ObjectReference;
use libc::c_void;

pub type SelectedPlan<'a> = SemiSpace<'a>;

pub const ALLOC_SS: Allocator = Allocator::Default;
pub const SCAN_BOOT_IMAGE: bool = true;

lazy_static! {
    pub static ref PLAN: SemiSpace<'static> = SemiSpace::new();
}

pub struct SemiSpace<'a> {
    pub control_collector_context: ControllerCollectorContext<'a>,
    hi: bool,
    pub copyspace0: CopySpace,
    pub copyspace1: CopySpace,
    ss_trace: Trace,
    pub versatile_space: ImmortalSpace,
}

impl<'a> Plan for SemiSpace<'a> {
    type MutatorT = SSMutator<'a>;
    type TraceLocalT = SSTraceLocal;
    type CollectorT = SSCollector<'a>;
    type ConstraintsT = SSConstraints;

    fn new() -> Self {
        SemiSpace {
            control_collector_context: ControllerCollectorContext::new(),
            hi: false,
            copyspace0: CopySpace::new(false),
            copyspace1: CopySpace::new(true),
            ss_trace: Trace::new(),
            versatile_space: ImmortalSpace::new(),
        }
    }

    fn gc_init(&self, heap_size: usize) {
        // FIXME
        default::gc_init(&self.copyspace0, heap_size / 3);
        self.copyspace1.init(heap_size / 3);
        self.versatile_space.init(heap_size / 3);
    }

    fn bind_mutator(&self, thread_id: usize) -> *mut c_void {
        default::bind_mutator(Self::MutatorT::new(thread_id, self.fromspace(), &self.versatile_space))
    }

    fn will_never_move(&self, object: ObjectReference) -> bool {
        if self.tospace().in_space(object) || self.fromspace().in_space(object) {
            return false;
        }

        if self.versatile_space.in_space(object) {
            return true;
        }

        // this preserves correctness over efficiency
        false
    }
}

impl<'a> SemiSpace<'a> {
    pub fn tospace(&self) -> &CopySpace {
        if self.hi {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
    }

    pub fn fromspace(&self) -> &CopySpace {
        if self.hi {
            &self.copyspace0
        } else {
            &self.copyspace1
        }
    }

    pub fn collection_phase(&mut self, phase: Phase) {
        if let Phase::Prepare = phase {
            self.hi = !self.hi;
            self.copyspace0.prepare(self.hi);
            self.copyspace1.prepare(!self.hi);
        }
    }
}