use ::vm::{ActivePlan, VMActivePlan};
use ::plan;
use ::plan::{Plan, MutatorContext, SelectedPlan, CollectorContext, ParallelCollector};

#[derive(Clone)]
#[derive(PartialEq)]
pub enum Schedule {
    Global,
    Collector,
    Mutator,
    Concurrent,
    Placeholder,
    Complex,
}

#[derive(Clone)]
#[derive(PartialEq)]
pub enum Phase {
    // Phases
    SetCollectionKind,
    Initiate,
    Prepare,
    PrepareStacks,
    StackRoots,
    Roots,
    Closure,
    SoftRefs,
    WeakRefs,
    Finalizable,
    WeakTrackRefs,
    PhantomRefs,
    Forward,
    ForwardRefs,
    ForwardFinalizable,
    Release,
    Complete,
    // Sanity placeholder
    PreSanityPlaceholder,
    PostSanityPlaceholder,
    // Sanity phases
    SanitySetPreGC,
    SanitySetPostGC,
    SanityPrepare,
    SanityRoots,
    SanityCopyRoots,
    SanityBuildTable,
    SanityCheckTable,
    SanityRelease,
    // Complex phases
    Complex(Vec<(Schedule, Phase)>),
    // No phases are left
    Empty,
}

// FIXME: It's probably unsafe to call most of these functions, because thread_id

pub fn begin_new_phase_stack(thread_id: usize, scheduled_phase: (Schedule, Phase)) {
    let order = unsafe { VMActivePlan::collector(thread_id).rendezvous() };

    if order == 0 {
        push_scheduled_phase(scheduled_phase);
    }

    process_phase_stack(thread_id, false);
}

pub fn continue_phase_stack(thread_id: usize) {
    process_phase_stack(thread_id, true);
}

pub fn process_phase_stack(thread_id: usize, resume: bool) {
    let plan = VMActivePlan::global();
    let collector = unsafe { VMActivePlan::collector(thread_id) };
    let order = collector.rendezvous();
    let primary = order == 0;
    if primary && resume {
        plan::plan::set_gc_status(plan::plan::GcStatus::GcProper);
    }
    let is_even_phase = true;
    if primary {
        // FIXME allowConcurrentPhase
        set_next_phase(false, get_next_phase(), false);
    }
    collector.rendezvous();
    let (mut schedule, mut phase) = get_current_phase(is_even_phase);
    while {
        let cp = get_current_phase(is_even_phase);
        schedule = cp.0;
        phase = cp.1;
        phase != Phase::Empty
    } {
        // FIXME timer
        match schedule {
            Schedule::Global => {
                if primary {
                    unsafe { plan.collection_phase(&phase) }
                }
            }
            Schedule::Collector => {
                collector.collection_phase(&phase, primary)
            }
            Schedule::Mutator => {
                while let Some(mutator) = VMActivePlan::get_next_mutator() {
                    mutator.collection_phase(&phase, primary);
                }
            }
            Schedule::Concurrent => {
                unimplemented!()
            }
            _ => {
                panic!("Invalid schedule in Phase.process_phase_stack")
            }
        }
    }
}

pub fn get_current_phase(is_even_phase: bool) -> (Schedule, Phase) {
    unimplemented!()
}

pub fn get_next_phase() -> (Schedule, Phase) {
    unimplemented!()
}

pub fn set_next_phase(is_even_phase: bool,
                      scheduled_phase: (Schedule, Phase),
                      needs_reset_rendezvous: bool) {
    unimplemented!()
}

pub fn push_scheduled_phase(scheduled_phase: (Schedule, Phase)) {
    unimplemented!()
}
