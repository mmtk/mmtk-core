use ::vm::{ActivePlan, VMActivePlan};
use ::plan;
use ::plan::{Plan, MutatorContext, SelectedPlan, CollectorContext, ParallelCollector};
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic;

#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum Schedule {
    Global,
    Collector,
    Mutator,
    Concurrent,
    Placeholder,
    Complex,
    Empty,
}

#[derive(Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
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
    Complex(Vec<(Schedule, Phase)>, usize),
    // associated cursor
    // No phases are left
    Empty,
}

static mut EVEN_SCHEDULED_PHASE: (Schedule, Phase) = (Schedule::Empty, Phase::Empty);
static mut ODD_SCHEDULED_PHASE: (Schedule, Phase) = (Schedule::Empty, Phase::Empty);
static mut EVEN_MUTATOR_RESET_RENDEZVOUS: bool = false;
static mut ODD_MUTATOR_RESET_RENDEZVOUS: bool = false;
static COMPLEX_PHASE_CURSOR: AtomicUsize = AtomicUsize::new(0);

lazy_static! {
    static ref PHASE_STACK: Mutex<Vec<(Schedule, Phase)>> = Mutex::new(vec![]);
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

fn process_phase_stack(thread_id: usize, resume: bool) {
    let mut resume = resume;
    let plan = VMActivePlan::global();
    let collector = unsafe { VMActivePlan::collector(thread_id) };
    let order = collector.rendezvous();
    let primary = order == 0;
    if primary && resume {
        plan::plan::set_gc_status(plan::plan::GcStatus::GcProper);
    }
    let mut is_even_phase = true;
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
                debug!("Execute {:?} as Global...", phase);
                if primary {
                    unsafe { plan.collection_phase(thread_id, &phase) }
                }
            }
            Schedule::Collector => {
                debug!("Execute {:?} as Collector...", phase);
                collector.collection_phase(thread_id, &phase, primary)
            }
            Schedule::Mutator => {
                debug!("Execute {:?} as Mutator...", phase);
                while let Some(mutator) = VMActivePlan::get_next_mutator() {
                    mutator.collection_phase(thread_id, &phase, primary);
                }
            }
            Schedule::Concurrent => {
                unimplemented!()
            }
            _ => {
                panic!("Invalid schedule in Phase.process_phase_stack")
            }
        }

        if primary {
            let next = get_next_phase();
            let needs_reset_rendezvous = next.1 != Phase::Empty && (schedule == Schedule::Mutator && next.0 == Schedule::Mutator);
            set_next_phase(is_even_phase, next, needs_reset_rendezvous);
        }

        collector.rendezvous();

        if primary && schedule == Schedule::Mutator {
            VMActivePlan::reset_mutator_iterator();
        }

        if needs_mutator_reset_rendevous(is_even_phase) {
            collector.rendezvous();
        }

        // FIXME timer
        is_even_phase = !is_even_phase;
        resume = false;
    }
}

fn get_current_phase(is_even_phase: bool) -> (Schedule, Phase) {
    unsafe { if is_even_phase { EVEN_SCHEDULED_PHASE.clone() } else { ODD_SCHEDULED_PHASE.clone() } }
}

fn get_next_phase() -> (Schedule, Phase) {
    let mut stack = PHASE_STACK.lock().unwrap();
    while !stack.is_empty() {
        let (schedule, mut phase) = stack.pop().unwrap();
        match schedule {
            Schedule::Placeholder => {}
            Schedule::Global => {
                return (schedule, phase);
            }
            Schedule::Collector => {
                return (schedule, phase);
            }
            Schedule::Mutator => {
                return (schedule, phase);
            }
            Schedule::Concurrent => {
                unimplemented!()
            }
            Schedule::Complex => {
                let mut internal_phase = (Schedule::Empty, Phase::Empty);
                // FIXME start complex timer
                if let Phase::Complex(ref v, ref mut cursor) = phase {
                    trace!("Complex phase: {:?} with cursor: {:?}", v, cursor);
                    if *cursor < v.len() {
                        internal_phase = v[*cursor].clone();
                        *cursor += 1;
                    } else {
                        trace!("Finished processing phase");
                    }
                } else {
                    panic!("Complex schedule should be paired with complex phase");
                }
                if internal_phase.1 != Phase::Empty {
                    stack.push((schedule, phase));
                    stack.push(internal_phase);
                }
                // FIXME stop complex timer
            }
            _ => {
                panic!("Invalid phase type encountered");
            }
        }
    }
    (Schedule::Empty, Phase::Empty)
}

fn set_next_phase(is_even_phase: bool,
                  scheduled_phase: (Schedule, Phase),
                  needs_reset_rendezvous: bool) {
    if is_even_phase {
        unsafe {
            ODD_SCHEDULED_PHASE = scheduled_phase;
            EVEN_MUTATOR_RESET_RENDEZVOUS = needs_reset_rendezvous;
        }
    } else {
        unsafe {
            EVEN_SCHEDULED_PHASE = scheduled_phase;
            ODD_MUTATOR_RESET_RENDEZVOUS = needs_reset_rendezvous;
        }
    }
}

pub fn push_scheduled_phase(scheduled_phase: (Schedule, Phase)) {
    PHASE_STACK.lock().unwrap().push(scheduled_phase);
}

fn needs_mutator_reset_rendevous(is_even_phase: bool) -> bool {
    unsafe { if is_even_phase { EVEN_MUTATOR_RESET_RENDEZVOUS } else { ODD_MUTATOR_RESET_RENDEZVOUS } }
}