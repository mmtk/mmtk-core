use libc::c_void;
use ::util::ObjectReference;
use super::{MutatorContext, CollectorContext, ParallelCollector, TraceLocal, phase, Phase};
use std::sync::atomic::{self, AtomicBool};

pub trait Plan {
    type MutatorT: MutatorContext;
    type TraceLocalT: TraceLocal;
    type CollectorT: ParallelCollector;

    fn new() -> Self;
    // unsafe because this can only be called once by the init thread
    unsafe fn gc_init(&self, heap_size: usize);
    fn bind_mutator(&self, thread_id: usize) -> *mut c_void;
    fn will_never_move(&self, object: ObjectReference) -> bool;
    // unsafe because only the primary collector thread can call this
    unsafe fn collection_phase(&self, thread_id: usize, phase: &phase::Phase);
}

#[derive(PartialEq)]
pub enum GcStatus {
    NotInGC,
    GcPrepare,
    GcProper,
}

static mut GC_STATUS: GcStatus = GcStatus::NotInGC;
pub static STACKS_PREPARED: AtomicBool = AtomicBool::new(false);


#[repr(i32)]
#[derive(Clone, Copy, Debug)]
pub enum Allocator {
    Default = 0,
    NonReference = 1,
    NonMoving = 2,
    Immortal = 3,
    Los = 4,
    PrimitiveLos = 5,
    GcSpy = 6,
    Code = 7,
    LargeCode = 8,
    Allocators = 9,
    DefaultSite = -1,
}

pub mod default {
    use std::thread;
    use libc::c_void;

    use ::policy::space::Space;
    use ::plan::mutator_context::MutatorContext;

    use ::util::heap::PageResource;

    use super::super::selected_plan::PLAN;

    pub fn gc_init<PR: PageResource<S>, S: Space<PR>>(space: &mut S) {
        space.init();

        if !cfg!(feature = "jikesrvm") {
            thread::spawn(|| {
                PLAN.control_collector_context.run(0);
            });
        }
    }

    pub fn bind_mutator<M: MutatorContext>(ctx: M) -> *mut c_void {
        Box::into_raw(Box::new(ctx)) as *mut c_void
    }
}

lazy_static! {
    pub static ref PREPARE_STACKS: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::PrepareStacks),
        (phase::Schedule::Global, phase::Phase::PrepareStacks)
    ], 0);

    pub static ref SANITY_BUILD_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanityPrepare),
        (phase::Schedule::Collector, phase::Phase::SanityPrepare),
        (phase::Schedule::Complex, PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::SanityRoots),
        (phase::Schedule::Global, phase::Phase::SanityRoots),
        (phase::Schedule::Collector, phase::Phase::SanityCopyRoots),
        (phase::Schedule::Global, phase::Phase::SanityBuildTable)
    ], 0);

    pub static ref SANITY_CHECK_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanityCheckTable),
        (phase::Schedule::Collector, phase::Phase::SanityRelease),
        (phase::Schedule::Global, phase::Phase::SanityRelease)
    ], 0);

    pub static ref INIT_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SetCollectionKind),
        (phase::Schedule::Global, phase::Phase::Initiate),
        (phase::Schedule::Placeholder, phase::Phase::PreSanityPlaceholder)
    ], 0);

    pub static ref ROOT_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::Prepare),
        (phase::Schedule::Global, phase::Phase::Prepare),
        (phase::Schedule::Collector, phase::Phase::Prepare),
        (phase::Schedule::Complex, PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global, phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global, phase::Phase::Roots),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Collector, phase::Phase::Closure)
    ], 0);

    pub static ref REF_TYPE_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Collector, phase::Phase::Closure),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Collector, phase::Phase::Closure),
        (phase::Schedule::Placeholder, phase::Phase::WeakTrackRefs),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs)
    ], 0);

    pub static ref FORWARD_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Placeholder, phase::Phase::Forward),
        (phase::Schedule::Collector, phase::Phase::ForwardRefs),
        (phase::Schedule::Collector, phase::Phase::ForwardFinalizable)
    ], 0);

    pub static ref COMPLETE_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::Release),
        (phase::Schedule::Collector, phase::Phase::Release),
        (phase::Schedule::Global, phase::Phase::Release)
    ], 0);

    pub static ref FINISH_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Placeholder, phase::Phase::PostSanityPlaceholder),
        (phase::Schedule::Collector, phase::Phase::Complete),
        (phase::Schedule::Global, phase::Phase::Complete)
    ], 0);

    pub static ref COLLECTION: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Complex, INIT_PHASE.clone()),
        (phase::Schedule::Complex, ROOT_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, REF_TYPE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, FORWARD_PHASE.clone()),
        (phase::Schedule::Complex, COMPLETE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, FINISH_PHASE.clone())
    ], 0);

    pub static ref PRE_SANITY_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanitySetPreGC),
        (phase::Schedule::Complex, SANITY_BUILD_PHASE.clone()),
        (phase::Schedule::Complex, SANITY_CHECK_PHASE.clone())
    ], 0);

    pub static ref POST_SANITY_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanitySetPostGC),
        (phase::Schedule::Complex, SANITY_BUILD_PHASE.clone()),
        (phase::Schedule::Complex, SANITY_CHECK_PHASE.clone())
    ], 0);
}

pub fn set_gc_status(s: GcStatus) {
    // FIXME
    unsafe { GC_STATUS = s };
}

pub fn stacks_prepared() -> bool {
    STACKS_PREPARED.load(atomic::Ordering::Relaxed)
}

pub fn gc_in_progress() -> bool {
    unsafe { GC_STATUS != GcStatus::NotInGC }
}

pub fn gc_in_progress_proper() -> bool {
    unsafe { GC_STATUS == GcStatus::GcProper }
}