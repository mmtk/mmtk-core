use ::plan;
use ::plan::{CollectorContext, MutatorContext, ParallelCollector, Plan, SelectedPlan};
use ::vm::{ActivePlan, VMActivePlan};
use std::sync::atomic;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use util::statistics::phase_timer::PhaseTimer;
use ::util::OpaquePointer;
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
    Complex(Vec<ScheduledPhase>, usize, Option<usize>),
    // associated cursor
    // No phases are left
    Empty,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ScheduledPhase {
    schedule: Schedule,
    phase: Phase,
}

impl ScheduledPhase {
    pub fn new(schedule: Schedule, phase: Phase) -> Self {
        ScheduledPhase { schedule, phase }
    }

    pub fn empty() -> Self {
        ScheduledPhase {
            schedule: Schedule::Empty,
            phase: Phase::Empty,
        }
    }
}

static EVEN_MUTATOR_RESET_RENDEZVOUS: AtomicBool = AtomicBool::new(false);
static ODD_MUTATOR_RESET_RENDEZVOUS: AtomicBool = AtomicBool::new(false);
static COMPLEX_PHASE_CURSOR: AtomicUsize = AtomicUsize::new(0);

lazy_static! {
    static ref PHASE_STACK: Mutex<Vec<ScheduledPhase>> = Mutex::new(vec![]);
    static ref EVEN_SCHEDULED_PHASE: Mutex<ScheduledPhase> = Mutex::new(ScheduledPhase::empty());
    static ref ODD_SCHEDULED_PHASE: Mutex<ScheduledPhase> = Mutex::new(ScheduledPhase::empty());
    static ref START_COMPLEX_TIMER: Mutex<Option<usize>> = Mutex::new(None);
    static ref STOP_COMPLEX_TIMER: Mutex<Option<usize>> = Mutex::new(None);
    static ref PHASE_TIMER: PhaseTimer = PhaseTimer::new();
}

// FIXME: It's probably unsafe to call most of these functions, because tls

pub fn begin_new_phase_stack(tls: OpaquePointer, scheduled_phase: ScheduledPhase) {
    let order = unsafe { VMActivePlan::collector(tls).rendezvous() };

    if order == 0 {
        push_scheduled_phase(scheduled_phase);
    }

    process_phase_stack(tls, false);
}

pub fn continue_phase_stack(tls: OpaquePointer) {
    process_phase_stack(tls, true);
}

fn resume_complex_timers() {
    let stack = PHASE_STACK.lock().unwrap();
    for cp in (*stack).iter().rev() {
        PHASE_TIMER.start_timer(&cp.phase);
    }
}

fn process_phase_stack(tls: OpaquePointer, resume: bool) {
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
        let schedule = cp.schedule;
        let phase = cp.phase;
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
            let needs_reset_rendezvous = next.phase != Phase::Empty && (schedule == Schedule::Mutator && next.schedule == Schedule::Mutator);
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

fn get_current_phase(is_even_phase: bool) -> ScheduledPhase {
    if is_even_phase {
        (*EVEN_SCHEDULED_PHASE.lock().unwrap()).clone()
    } else {
        (*ODD_SCHEDULED_PHASE.lock().unwrap()).clone()
    }
}

fn get_next_phase() -> ScheduledPhase {
    let mut stack = PHASE_STACK.lock().unwrap();
    while !stack.is_empty() {
        let mut scheduled_phase = stack.pop().unwrap();
        match scheduled_phase.schedule {
            Schedule::Placeholder => {}
            Schedule::Global => {
                return scheduled_phase;
            }
            Schedule::Collector => {
                return scheduled_phase;
            }
            Schedule::Mutator => {
                return scheduled_phase;
            }
            Schedule::Concurrent => {
                unimplemented!()
            }
            Schedule::Complex => {
                let mut internal_phase = ScheduledPhase::empty();
                // FIXME start complex timer
                if let Phase::Complex(ref v, ref mut cursor, ref timer_id) = scheduled_phase.phase {
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
                if internal_phase.phase != Phase::Empty {
                    stack.push(scheduled_phase);
                    stack.push(internal_phase);
                }
                // FIXME stop complex timer
            }
            _ => {
                panic!("Invalid phase type encountered");
            }
        }
    }
    ScheduledPhase::empty()
}

fn set_next_phase(is_even_phase: bool,
                  scheduled_phase: ScheduledPhase,
                  needs_reset_rendezvous: bool) {
    if is_even_phase {
        *ODD_SCHEDULED_PHASE.lock().unwrap() = scheduled_phase;
        EVEN_MUTATOR_RESET_RENDEZVOUS.store(needs_reset_rendezvous, Ordering::Relaxed);
    } else {
        *EVEN_SCHEDULED_PHASE.lock().unwrap() = scheduled_phase;
        ODD_MUTATOR_RESET_RENDEZVOUS.store(needs_reset_rendezvous, Ordering::Relaxed);
    }
}

pub fn push_scheduled_phase(scheduled_phase: ScheduledPhase) {
    PHASE_STACK.lock().unwrap().push(scheduled_phase);
}

fn needs_mutator_reset_rendevous(is_even_phase: bool) -> bool {
    if is_even_phase {
        EVEN_MUTATOR_RESET_RENDEZVOUS.load(Ordering::Relaxed)
    } else {
        ODD_MUTATOR_RESET_RENDEZVOUS.load(Ordering::Relaxed)
    }
}