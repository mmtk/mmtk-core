use ::plan;
use ::plan::{CollectorContext, MutatorContext, ParallelCollector, Plan, SelectedPlan};
use ::vm::{ActivePlan, VMActivePlan};
use std::sync::atomic;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use util::statistics::phase_timer::PhaseTimer;

use libc::c_void;

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
    // G1 phases
    CollectionSetSelection,
    EvacuatePrepare,
    EvacuateClosure,
    EvacuateRelease,
    // Complex phases
    Complex(Vec<(Schedule, Phase)>, usize, Option<usize>),
    // associated cursor
    // No phases are left
    Empty,
}

static EVEN_MUTATOR_RESET_RENDEZVOUS: AtomicBool = AtomicBool::new(false);
static ODD_MUTATOR_RESET_RENDEZVOUS: AtomicBool = AtomicBool::new(false);
static COMPLEX_PHASE_CURSOR: AtomicUsize = AtomicUsize::new(0);

lazy_static! {
    static ref PHASE_STACK: Mutex<Vec<(Schedule, Phase)>> = Mutex::new(vec![]);
    static ref EVEN_SCHEDULED_PHASE: Mutex<(Schedule, Phase)> = Mutex::new((Schedule::Empty, Phase::Empty));
    static ref ODD_SCHEDULED_PHASE: Mutex<(Schedule, Phase)> = Mutex::new((Schedule::Empty, Phase::Empty));
    static ref START_COMPLEX_TIMER: Mutex<Option<usize>> = Mutex::new(None);
    static ref STOP_COMPLEX_TIMER: Mutex<Option<usize>> = Mutex::new(None);
    static ref PHASE_TIMER: PhaseTimer = PhaseTimer::new();
}

// FIXME: It's probably unsafe to call most of these functions, because tls

pub fn begin_new_phase_stack(tls: *mut c_void, scheduled_phase: (Schedule, Phase)) {
    let order = unsafe { VMActivePlan::collector(tls).rendezvous() };

    if order == 0 {
        push_scheduled_phase(scheduled_phase);
    }

    process_phase_stack(tls, false);
}

pub fn continue_phase_stack(tls: *mut c_void) {
    process_phase_stack(tls, true);
}

fn resume_complex_timers() {
    unimplemented!()
}

fn process_phase_stack(tls: *mut c_void, resume: bool) {
    let mut resume = resume;
    let plan = VMActivePlan::global();
    let collector = unsafe { VMActivePlan::collector(tls) };
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
    loop {
        let cp = get_current_phase(is_even_phase);
        let schedule = cp.0;
        let phase = cp.1;
        if phase == Phase::Empty {
            break;
        }
        if primary {
            if resume {
                resume_complex_timers();
            }
            PHASE_TIMER.start_timer(&phase);
            {
                let mut start_complex_timer = START_COMPLEX_TIMER.lock().unwrap();
                if let Some(id) = *start_complex_timer {
                    PHASE_TIMER.start_timer_id(id);
                    *start_complex_timer = None;
                }
            }
        }
        match schedule {
            Schedule::Global => {
                debug!("Execute {:?} as Global...", phase);
                if primary {
                    unsafe { plan.collection_phase(tls, &phase) }
                }
            }
            Schedule::Collector => {
                debug!("Execute {:?} as Collector...", phase);
                collector.collection_phase(tls, &phase, primary)
            }
            Schedule::Mutator => {
                debug!("Execute {:?} as Mutator...", phase);
                while let Some(mutator) = VMActivePlan::get_next_mutator() {
                    mutator.collection_phase(tls, &phase, primary);
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

        if primary {
            PHASE_TIMER.stop_timer(&phase);
            {
                let mut stop_complex_timer = STOP_COMPLEX_TIMER.lock().unwrap();
                if let Some(id) = *stop_complex_timer {
                    PHASE_TIMER.stop_timer_id(id);
                    *stop_complex_timer = None;
                }
            }
        }
        is_even_phase = !is_even_phase;
        resume = false;
    }
}

fn get_current_phase(is_even_phase: bool) -> (Schedule, Phase) {
    if is_even_phase {
        (*EVEN_SCHEDULED_PHASE.lock().unwrap()).clone()
    } else {
        (*ODD_SCHEDULED_PHASE.lock().unwrap()).clone()
    }
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
                if let Phase::Complex(ref v, ref mut cursor, ref timer_id) = phase {
                    trace!("Complex phase: {:?} with cursor: {:?}", v, cursor);
                    if *cursor == 0 {
                        if let Some(id) = timer_id {
                            let mut start_complex_timer = START_COMPLEX_TIMER.lock().unwrap();
                            *start_complex_timer = Some(*id);
                        }
                    }
                    if *cursor < v.len() {
                        internal_phase = v[*cursor].clone();
                        *cursor += 1;
                    } else {
                        if let Some(id) = timer_id {
                            let mut stop_complex_timer = STOP_COMPLEX_TIMER.lock().unwrap();
                            *stop_complex_timer = Some(*id);
                        }
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
        *ODD_SCHEDULED_PHASE.lock().unwrap() = scheduled_phase;
        EVEN_MUTATOR_RESET_RENDEZVOUS.store(needs_reset_rendezvous, Ordering::Relaxed);
    } else {
        *EVEN_SCHEDULED_PHASE.lock().unwrap() = scheduled_phase;
        ODD_MUTATOR_RESET_RENDEZVOUS.store(needs_reset_rendezvous, Ordering::Relaxed);
    }
}

pub fn push_scheduled_phase(scheduled_phase: (Schedule, Phase)) {
    PHASE_STACK.lock().unwrap().push(scheduled_phase);
}

fn needs_mutator_reset_rendevous(is_even_phase: bool) -> bool {
    if is_even_phase {
        EVEN_MUTATOR_RESET_RENDEZVOUS.load(Ordering::Relaxed)
    } else {
        ODD_MUTATOR_RESET_RENDEZVOUS.load(Ordering::Relaxed)
    }
}