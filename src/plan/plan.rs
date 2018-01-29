use libc::c_void;
use ::util::ObjectReference;
use ::plan::{MutatorContext, CollectorContext, ParallelCollector, TraceLocal};
use ::plan::phase;

pub trait Plan {
    type MutatorT: MutatorContext;
    type TraceLocalT: TraceLocal;
    type CollectorT: ParallelCollector;

    fn new() -> Self;
    fn gc_init(&self, heap_size: usize);
    fn bind_mutator(&self, thread_id: usize) -> *mut c_void;
    fn will_never_move(&self, object: ObjectReference) -> bool;
}

#[repr(i32)]
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

    use super::super::selected_plan::PLAN;

    pub fn gc_init<T: Space>(space: &T, heap_size: usize) {
        space.init(heap_size);

        if !cfg!(feature = "jikesrvm") {
            thread::spawn(|| {
                PLAN.control_collector_context.run(0);
            });
        }
    }

    pub fn bind_mutator<T: MutatorContext>(ctx: T) -> *mut c_void {
        Box::into_raw(Box::new(ctx)) as *mut c_void
    }
}

lazy_static! {
    pub static ref PREPARE_STACKS: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::PrepareStacks),
        (phase::Schedule::Global, phase::Phase::PrepareStacks)
    ]);

    pub static ref SANITY_BUILD_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanityPrepare),
        (phase::Schedule::Collector, phase::Phase::SanityPrepare),
        (phase::Schedule::Complex, PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::SanityRoots),
        (phase::Schedule::Global, phase::Phase::SanityRoots),
        (phase::Schedule::Collector, phase::Phase::SanityCopyRoots),
        (phase::Schedule::Global, phase::Phase::SanityBuildTable)
    ]);

    pub static ref SANITY_CHECK_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanityCheckTable),
        (phase::Schedule::Collector, phase::Phase::SanityRelease),
        (phase::Schedule::Global, phase::Phase::SanityRelease)
    ]);

    pub static ref INIT_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SetCollectionKind),
        (phase::Schedule::Global, phase::Phase::Initiate),
        (phase::Schedule::Placeholder, phase::Phase::PreSanityPlaceholder)
    ]);

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
    ]);

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
    ]);

    pub static ref FORWARD_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Placeholder, phase::Phase::Forward),
        (phase::Schedule::Collector, phase::Phase::ForwardRefs),
        (phase::Schedule::Collector, phase::Phase::ForwardFinalizable)
    ]);

    pub static ref COMPLETE_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::Release),
        (phase::Schedule::Collector, phase::Phase::Release),
        (phase::Schedule::Global, phase::Phase::Release)
    ]);

    pub static ref FINISH_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Placeholder, phase::Phase::PostSanityPlaceholder),
        (phase::Schedule::Collector, phase::Phase::Complete),
        (phase::Schedule::Global, phase::Phase::Complete)
    ]);

    pub static ref COLLECTION: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Complex, INIT_PHASE.clone()),
        (phase::Schedule::Complex, ROOT_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, REF_TYPE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, FORWARD_PHASE.clone()),
        (phase::Schedule::Complex, COMPLETE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, FINISH_PHASE.clone())
    ]);

    pub static ref PRE_SANITY_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanitySetPreGC),
        (phase::Schedule::Complex, SANITY_BUILD_PHASE.clone()),
        (phase::Schedule::Complex, SANITY_CHECK_PHASE.clone())
    ]);

    pub static ref POST_SANITY_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global, phase::Phase::SanitySetPostGC),
        (phase::Schedule::Complex, SANITY_BUILD_PHASE.clone()),
        (phase::Schedule::Complex, SANITY_CHECK_PHASE.clone())
    ]);
}