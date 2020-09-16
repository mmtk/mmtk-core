use crate::plan;
use crate::plan::phase::Phase::*;
use crate::plan::phase::Schedule::*;
use crate::plan::{CollectorContext, MutatorContext, ParallelCollector, Plan};
use crate::util::statistics::phase_timer::PhaseTimer;
use crate::util::statistics::stats::Stats;
use crate::util::statistics::{Counter, Timer};
use crate::util::OpaquePointer;
use crate::vm::ActivePlan;
use crate::vm::VMBinding;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Clone, PartialEq, Debug)]
pub enum Schedule {
    Global,
    Collector,
    Mutator,
    Concurrent,
    Placeholder,
    Complex,
    Empty,
}

#[derive(Clone, Debug)]
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
    Complex(Vec<ScheduledPhase>, usize, Option<Arc<Mutex<Timer>>>),
    // associated cursor
    // No phases are left
    Empty,
}

impl Phase {
    pub fn is_empty(&self) -> bool {
        match *self {
            Phase::Empty => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScheduledPhase {
    schedule: Schedule,
    phase: Phase,
}

impl ScheduledPhase {
    pub const EMPTY: Self = ScheduledPhase {
        schedule: Schedule::Empty,
        phase: Phase::Empty,
    };

    pub fn new(schedule: Schedule, phase: Phase) -> Self {
        ScheduledPhase { schedule, phase }
    }
}

pub struct PhaseManager {
    even_mutator_reset_rendezvous: AtomicBool,
    odd_mutator_reset_rendezvous: AtomicBool,

    phase_stack: Mutex<Vec<ScheduledPhase>>,
    even_scheduled_phase: Mutex<ScheduledPhase>,
    odd_scheduled_phase: Mutex<ScheduledPhase>,
    start_complex_timer: Mutex<Option<Arc<Mutex<Timer>>>>,
    stop_complex_timer: Mutex<Option<Arc<Mutex<Timer>>>>,
    phase_timer: PhaseTimer,

    // TODO: Some plan may want to change the phase. We need to figure out a pattern to allow it.
    pub collection_phase: Phase,
}

impl PhaseManager {
    pub fn new(stats: &Stats) -> Self {
        PhaseManager {
            even_mutator_reset_rendezvous: AtomicBool::new(false),
            odd_mutator_reset_rendezvous: AtomicBool::new(false),

            phase_stack: Mutex::new(vec![]),
            even_scheduled_phase: Mutex::new(ScheduledPhase::EMPTY),
            odd_scheduled_phase: Mutex::new(ScheduledPhase::EMPTY),
            start_complex_timer: Mutex::new(None),
            stop_complex_timer: Mutex::new(None),
            phase_timer: PhaseTimer::new(stats),

            collection_phase: PhaseManager::define_phase_collection(stats),
        }
    }

    fn define_phase_prepare_stacks(_stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Mutator, PrepareStacks),
                ScheduledPhase::new(Global, PrepareStacks),
            ],
            0,
            None,
        )
    }

    fn define_phase_init(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Global, SetCollectionKind),
                ScheduledPhase::new(Global, Initiate),
                ScheduledPhase::new(Placeholder, PreSanityPlaceholder),
            ],
            0,
            Some(stats.new_timer("init", false, true)),
        )
    }

    fn define_phase_root_closure(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Mutator, Prepare),
                ScheduledPhase::new(Global, Prepare),
                ScheduledPhase::new(Collector, Prepare),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_prepare_stacks(stats),
                ),
                ScheduledPhase::new(Collector, StackRoots),
                ScheduledPhase::new(Global, StackRoots),
                ScheduledPhase::new(Collector, Roots),
                ScheduledPhase::new(Global, Roots),
                ScheduledPhase::new(Global, Closure),
                ScheduledPhase::new(Collector, Closure),
            ],
            0,
            None,
        )
    }

    fn define_phase_ref_type_closure(_stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Collector, SoftRefs),
                ScheduledPhase::new(Global, Closure),
                ScheduledPhase::new(Collector, Closure),
                ScheduledPhase::new(Collector, WeakRefs),
                ScheduledPhase::new(Collector, Finalizable),
                ScheduledPhase::new(Global, Closure),
                ScheduledPhase::new(Collector, Closure),
                ScheduledPhase::new(Placeholder, WeakTrackRefs),
                ScheduledPhase::new(Collector, PhantomRefs),
            ],
            0,
            None,
        )
    }

    fn define_phase_forward(_stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Placeholder, Forward),
                ScheduledPhase::new(Collector, ForwardRefs),
                ScheduledPhase::new(Collector, ForwardFinalizable),
            ],
            0,
            None,
        )
    }

    fn define_phase_complete_closure(_stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Mutator, Release),
                ScheduledPhase::new(Collector, Release),
                ScheduledPhase::new(Global, Release),
            ],
            0,
            None,
        )
    }

    fn define_phase_finish(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Placeholder, PostSanityPlaceholder),
                ScheduledPhase::new(Collector, Complete),
                ScheduledPhase::new(Global, Complete),
            ],
            0,
            Some(stats.new_timer("finish", false, true)),
        )
    }

    fn define_phase_collection(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Schedule::Complex, PhaseManager::define_phase_init(stats)),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_root_closure(stats),
                ),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_ref_type_closure(stats),
                ),
                ScheduledPhase::new(Schedule::Complex, PhaseManager::define_phase_forward(stats)),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_complete_closure(stats),
                ),
                ScheduledPhase::new(Schedule::Complex, PhaseManager::define_phase_finish(stats)),
            ],
            0,
            None,
        )
    }

    // It seems we never use the following functions for defining sanity checking phases.
    // FIXME: We need to check if sanity cehcking works or not.

    #[allow(unused)]
    fn define_phase_sanity_build(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Global, SanityPrepare),
                ScheduledPhase::new(Collector, SanityPrepare),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_prepare_stacks(stats),
                ),
                ScheduledPhase::new(Collector, SanityRoots),
                ScheduledPhase::new(Global, SanityRoots),
                ScheduledPhase::new(Collector, SanityCopyRoots),
                ScheduledPhase::new(Global, SanityBuildTable),
            ],
            0,
            None,
        )
    }

    #[allow(unused)]
    fn define_phase_sanity_check(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Global, SanityCheckTable),
                ScheduledPhase::new(Collector, SanityRelease),
                ScheduledPhase::new(Global, SanityRelease),
            ],
            0,
            None,
        )
    }

    #[allow(unused)]
    fn define_phase_pre_sanity(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Global, SanitySetPreGC),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_sanity_build(stats),
                ),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_sanity_check(stats),
                ),
            ],
            0,
            None,
        )
    }

    #[allow(unused)]
    fn define_phase_post_sanity(stats: &Stats) -> Phase {
        Phase::Complex(
            vec![
                ScheduledPhase::new(Global, SanitySetPostGC),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_sanity_build(stats),
                ),
                ScheduledPhase::new(
                    Schedule::Complex,
                    PhaseManager::define_phase_sanity_check(stats),
                ),
            ],
            0,
            None,
        )
    }

    // FIXME: It's probably unsafe to call most of these functions, because tls
    pub fn begin_new_phase_stack<VM: VMBinding>(
        &self,
        tls: OpaquePointer,
        scheduled_phase: ScheduledPhase,
    ) {
        unreachable!()
    }

    pub fn continue_phase_stack<VM: VMBinding>(&self, tls: OpaquePointer) {
        self.process_phase_stack::<VM>(tls, true);
    }

    fn resume_complex_timers(&self) {
        let stack = self.phase_stack.lock().unwrap();
        for cp in (*stack).iter().rev() {
            self.phase_timer.start_timer(&cp.phase);
        }
    }

    fn process_phase_stack<VM: VMBinding>(&self, tls: OpaquePointer, resume: bool) {
        unreachable!()
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
                Schedule::Concurrent => unimplemented!(),
                Schedule::Complex => {
                    let mut internal_phase = ScheduledPhase::EMPTY;
                    // FIXME start complex timer
                    if let Phase::Complex(ref v, ref mut cursor, ref timer_opt) =
                        scheduled_phase.phase
                    {
                        trace!("Complex phase: {:?} with cursor: {:?}", v, cursor);
                        if *cursor == 0 {
                            if let Some(ref t) = timer_opt {
                                let mut start_complex_timer =
                                    self.start_complex_timer.lock().unwrap();
                                *start_complex_timer = Some(t.clone());
                            }
                        }
                        if *cursor < v.len() {
                            internal_phase = v[*cursor].clone();
                            *cursor += 1;
                        } else {
                            if let Some(ref t) = timer_opt {
                                let mut stop_complex_timer =
                                    self.stop_complex_timer.lock().unwrap();
                                *stop_complex_timer = Some(t.clone());
                            }
                            trace!("Finished processing phase");
                        }
                    } else {
                        panic!("Complex schedule should be paired with complex phase");
                    }
                    if !internal_phase.phase.is_empty() {
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
        ScheduledPhase::EMPTY
    }

    fn set_next_phase(
        &self,
        is_even_phase: bool,
        scheduled_phase: ScheduledPhase,
        needs_reset_rendezvous: bool,
    ) {
        if is_even_phase {
            *self.odd_scheduled_phase.lock().unwrap() = scheduled_phase;
            self.even_mutator_reset_rendezvous
                .store(needs_reset_rendezvous, Ordering::Relaxed);
        } else {
            *self.even_scheduled_phase.lock().unwrap() = scheduled_phase;
            self.odd_mutator_reset_rendezvous
                .store(needs_reset_rendezvous, Ordering::Relaxed);
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
