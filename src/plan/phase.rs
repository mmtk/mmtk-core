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

pub struct PhaseManager {
    even_mutator_reset_rendezvous: AtomicBool,
    odd_mutator_reset_rendezvous: AtomicBool,
    complex_phase_cursor: AtomicUsize,

    phase_stack: Mutex<Vec<ScheduledPhase>>,
    even_scheduled_phase: Mutex<ScheduledPhase>,
    odd_scheduled_phase: Mutex<ScheduledPhase>,
    start_complex_timer: Mutex<Option<usize>>,
    stop_complex_timer: Mutex<Option<usize>>,
    phase_timer: PhaseTimer,
}

impl PhaseManager {
    pub fn new() -> Self {
        PhaseManager {
            even_mutator_reset_rendezvous: AtomicBool::new(false),
            odd_mutator_reset_rendezvous: AtomicBool::new(false),
            complex_phase_cursor: AtomicUsize::new(0),

            phase_stack: Mutex::new(vec![]),
            even_scheduled_phase: Mutex::new(ScheduledPhase::empty()),
            odd_scheduled_phase: Mutex::new(ScheduledPhase::empty()),
            start_complex_timer: Mutex::new(None),
            stop_complex_timer: Mutex::new(None),
            phase_timer: PhaseTimer::new(),
        }
    }

    // FIXME: It's probably unsafe to call most of these functions, because tls
    pub fn begin_new_phase_stack(&self, tls: OpaquePointer, scheduled_phase: ScheduledPhase) {
        let order = unsafe { VMActivePlan::collector(tls).rendezvous() };

        if order == 0 {
            self.push_scheduled_phase(scheduled_phase);
        }

        self.process_phase_stack(tls, false);
    }

    pub fn continue_phase_stack(&self, tls: OpaquePointer) {
        self.process_phase_stack(tls, true);
    }

    fn resume_complex_timers(&self) {
        let stack = self.phase_stack.lock().unwrap();
        for cp in (*stack).iter().rev() {
            self.phase_timer.start_timer(&cp.phase);
        }
    }

    fn process_phase_stack(&self, tls: OpaquePointer, resume: bool) {
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
            let next_phase = self.get_next_phase();
            self.set_next_phase(false, next_phase, false);
        }
        collector.rendezvous();
        loop {
            let cp = self.get_current_phase(is_even_phase);
            let schedule = cp.schedule;
            let phase = cp.phase;
            if phase == Phase::Empty {
                break;
            }
            if primary {
                if resume {
                    self.resume_complex_timers();
                }
                self.phase_timer.start_timer(&phase);
                {
                    let mut start_complex_timer = self.start_complex_timer.lock().unwrap();
                    if let Some(id) = *start_complex_timer {
                        self.phase_timer.start_timer_id(id);
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
                let next = self.get_next_phase();
                let needs_reset_rendezvous = next.phase != Phase::Empty && (schedule == Schedule::Mutator && next.schedule == Schedule::Mutator);
                self.set_next_phase(is_even_phase, next, needs_reset_rendezvous);
            }

            collector.rendezvous();

            if primary && schedule == Schedule::Mutator {
                VMActivePlan::reset_mutator_iterator();
            }

            if self.needs_mutator_reset_rendevous(is_even_phase) {
                collector.rendezvous();
            }

            if primary {
                self.phase_timer.stop_timer(&phase);
                {
                    let mut stop_complex_timer = self.stop_complex_timer.lock().unwrap();
                    if let Some(id) = *stop_complex_timer {
                        self.phase_timer.stop_timer_id(id);
                        *stop_complex_timer = None;
                    }
                }
            }
            is_even_phase = !is_even_phase;
            resume = false;
        }
    }

    fn get_current_phase(&self, is_even_phase: bool) -> ScheduledPhase {
        if is_even_phase {
            (*self.even_scheduled_phase.lock().unwrap()).clone()
        } else {
            (*self.odd_scheduled_phase.lock().unwrap()).clone()
        }
    }

    fn get_next_phase(&self) -> ScheduledPhase {
        let mut stack = self.phase_stack.lock().unwrap();
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
                                let mut start_complex_timer = self.start_complex_timer.lock().unwrap();
                                *start_complex_timer = Some(*id);
                            }
                        }
                        if *cursor < v.len() {
                            internal_phase = v[*cursor].clone();
                            *cursor += 1;
                        } else {
                            if let Some(id) = timer_id {
                                let mut stop_complex_timer = self.stop_complex_timer.lock().unwrap();
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

    fn set_next_phase(&self, is_even_phase: bool,
                      scheduled_phase: ScheduledPhase,
                      needs_reset_rendezvous: bool) {
        if is_even_phase {
            *self.odd_scheduled_phase.lock().unwrap() = scheduled_phase;
            self.even_mutator_reset_rendezvous.store(needs_reset_rendezvous, Ordering::Relaxed);
        } else {
            *self.even_scheduled_phase.lock().unwrap() = scheduled_phase;
            self.odd_mutator_reset_rendezvous.store(needs_reset_rendezvous, Ordering::Relaxed);
        }
    }

    pub fn push_scheduled_phase(&self, scheduled_phase: ScheduledPhase) {
        self.phase_stack.lock().unwrap().push(scheduled_phase);
    }

    fn needs_mutator_reset_rendevous(&self, is_even_phase: bool) -> bool {
        if is_even_phase {
            self.even_mutator_reset_rendezvous.load(Ordering::Relaxed)
        } else {
            self.odd_mutator_reset_rendezvous.load(Ordering::Relaxed)
        }
    }
}
