use ::plan;
use ::plan::{CollectorContext, MutatorContext, ParallelCollector, Plan, SelectedPlan};
use ::vm::{ActivePlan, VMActivePlan};
use std::sync::atomic;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

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
    RemSetRoots,
    EvacuatePrepare,
    EvacuateClosure,
    EvacuateRelease,
    RefineCards,
    // Validation phases
    ValidatePlaceholder,
    ValidatePrepare,
    ValidateClosure,
    ValidateRelease,
    // Concurrent gc related phases
    FlushCollector,
    FlushMutator,
    SetBarrierActive,
    ClearBarrierActive,
    FinalClosure,
    // Complex phases
    Complex(Vec<(Schedule, Phase)>, usize),
    Concurrent(Box<(Schedule, Phase)>),
    // associated cursor
    // No phases are left
    Empty,
}

static EVEN_MUTATOR_RESET_RENDEZVOUS: AtomicBool = AtomicBool::new(false);
static ODD_MUTATOR_RESET_RENDEZVOUS: AtomicBool = AtomicBool::new(false);
static COMPLEX_PHASE_CURSOR: AtomicUsize = AtomicUsize::new(0);
static mut CONCURRENT_PHASE: Phase = Phase::Empty;
static ALLOW_CONCURRENT_PHASE: AtomicBool = AtomicBool::new(false);

lazy_static! {
    static ref PHASE_STACK: Mutex<Vec<(Schedule, Phase)>> = Mutex::new(vec![]);
    static ref EVEN_SCHEDULED_PHASE: Mutex<(Schedule, Phase)> = Mutex::new((Schedule::Empty, Phase::Empty));
    static ref ODD_SCHEDULED_PHASE: Mutex<(Schedule, Phase)> = Mutex::new((Schedule::Empty, Phase::Empty));
}

pub fn get_concurrent_phase() -> Phase {
    unsafe { CONCURRENT_PHASE.clone() }
}

pub fn clear_concurrent_phase() {
    unsafe { CONCURRENT_PHASE = Phase::Empty };// _ID.store(0, Ordering::Relaxed);
}

pub fn concurrent_phase_active() -> bool {
    unsafe { CONCURRENT_PHASE != Phase::Empty }
}

pub fn notify_concurrent_phase_complete() -> bool {
    /* Concurrent phase is complete*/
    unsafe { CONCURRENT_PHASE = Phase::Empty };
    /* Remove it from the stack */
    { // popScheduledPhase()
        let mut stack = PHASE_STACK.lock().unwrap();
        stack.pop().unwrap();
    }
    /* Pop the next phase off the stack */
    let (schedule, next_scheduled_phase) = get_next_phase();
    if next_scheduled_phase != Phase::Empty {
      /* A concurrent phase, lets wake up and do it all again */
      if schedule == Schedule::Concurrent {
        unsafe { CONCURRENT_PHASE = next_scheduled_phase };
        return true;
      }
      /* Push phase back on and resume atomic collection */
      push_scheduled_phase((schedule, next_scheduled_phase));
      ::plan::SelectedPlan::trigger_internal_collection_request();
    } 
    return false;
  }

pub fn notify_concurrent_phase_incomplete() {

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

pub fn is_phase_stack_empty() -> bool {
    let stack = PHASE_STACK.lock().unwrap();
    return stack.is_empty();
}

fn process_phase_stack(tls: *mut c_void, resume: bool) {
    let mut resume = resume;
    let plan = VMActivePlan::global();
    let collector = unsafe { VMActivePlan::collector(tls) };
    let order = collector.rendezvous();
    let primary = order == 0;
    if primary && resume {
        debug_assert!(!is_phase_stack_empty());
        debug_assert!(!super::plan::gc_in_progress());
        plan::plan::set_gc_status(plan::plan::GcStatus::GcProper);
    }
    let mut is_even_phase = true;
    if primary {
        // FIXME allowConcurrentPhase
        ALLOW_CONCURRENT_PHASE.store(<SelectedPlan as Plan>::is_internal_triggered_collection() && !<SelectedPlan as Plan>::is_emergency_collection(), Ordering::Relaxed);
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
        // FIXME timer
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
                if primary {
                    unsafe { CONCURRENT_PHASE = phase.clone() };
                    /* Concurrent phase, we need to stop gc */
                    ::plan::plan::set_gc_status(plan::plan::GcStatus::NotInGC);
                    ::plan::plan::CONTROL_COLLECTOR_CONTEXT.request_concurrent_collection();
                    
                }
                collector.rendezvous();
                return;
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
                if !ALLOW_CONCURRENT_PHASE.load(Ordering::Relaxed) {
                    match phase {
                        Phase::Concurrent(alt) => {
                            stack.push(*alt.clone());
                            continue;
                        },
                        _ => unreachable!(),
                    }
                } else {
                    stack.push((schedule.clone(), phase.clone()));
                }
                return (schedule, phase);
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