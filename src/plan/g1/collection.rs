use ::plan::phase;
use ::plan::plan;

lazy_static! {
    static ref PREEMPT_CONCURRENT_CLOSURE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::FlushMutator),
        (phase::Schedule::Collector, phase::Phase::Closure),
    ], 0);

    static ref CONCURRENT_CLOSURE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Global,     phase::Phase::SetBarrierActive),
        (phase::Schedule::Mutator,    phase::Phase::SetBarrierActive),
        (phase::Schedule::Collector,  phase::Phase::FlushCollector),
        (phase::Schedule::Concurrent, phase::Phase::Concurrent(
          box (phase::Schedule::Complex, PREEMPT_CONCURRENT_CLOSURE.clone())
        )),
        (phase::Schedule::Global,     phase::Phase::ClearBarrierActive),
        (phase::Schedule::Mutator,    phase::Phase::ClearBarrierActive),
        (phase::Schedule::Mutator,    phase::Phase::FinalClosure),
        (phase::Schedule::Collector,  phase::Phase::FinalClosure),
    ], 0);

    static ref ROOT_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator, phase::Phase::Prepare),
        (phase::Schedule::Global, phase::Phase::Prepare),
        (phase::Schedule::Collector, phase::Phase::Prepare),
        (phase::Schedule::Complex, plan::PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global, phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global, phase::Phase::Roots),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Complex, CONCURRENT_CLOSURE.clone()),
    ], 0);

    static ref REF_TYPE_CLOSURE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Complex, CONCURRENT_CLOSURE.clone()),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Global, phase::Phase::Closure),
        (phase::Schedule::Complex, CONCURRENT_CLOSURE.clone()),
        (phase::Schedule::Placeholder, phase::Phase::WeakTrackRefs),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs)
    ], 0);

    static ref FULL_TRACE_EVACUATE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator,   phase::Phase::EvacuatePrepare),
        (phase::Schedule::Global,    phase::Phase::EvacuatePrepare),
        (phase::Schedule::Collector, phase::Phase::EvacuatePrepare),
        
        // (phase::Schedule::Mutator,   phase::Phase::RefineCards),
        // (phase::Schedule::Global,    phase::Phase::RefineCards),
        // (phase::Schedule::Collector, phase::Phase::RefineCards),
        // Roots
        (phase::Schedule::Complex,   plan::PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global,    phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global,    phase::Phase::Roots),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        // Refs
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs),

        (phase::Schedule::Mutator,   phase::Phase::EvacuateRelease),
        (phase::Schedule::Global,    phase::Phase::EvacuateRelease),
        (phase::Schedule::Collector, phase::Phase::EvacuateRelease),
    ], 0);

    static ref REMSET_EVACUATE_PHASE: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Mutator,   phase::Phase::EvacuatePrepare),
        (phase::Schedule::Global,    phase::Phase::EvacuatePrepare),
        (phase::Schedule::Collector, phase::Phase::EvacuatePrepare),
        
        // (phase::Schedule::Mutator,   phase::Phase::RefineCards),
        // (phase::Schedule::Global,    phase::Phase::RefineCards),
        // (phase::Schedule::Collector, phase::Phase::RefineCards),
        // Roots
        (phase::Schedule::Complex,   plan::PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global,    phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global,    phase::Phase::Roots),
        (phase::Schedule::Mutator,   phase::Phase::RefineCards),
        (phase::Schedule::Global,    phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RemSetRoots),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),

        // (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        // (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        // Refs
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs),

        // ForwardRefs
        // (phase::Schedule::Complex, plan::FORWARD_PHASE.clone()),
        
        (phase::Schedule::Mutator,   phase::Phase::RefineCards),
        (phase::Schedule::Global,    phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RemSetRoots),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),

        (phase::Schedule::Mutator,   phase::Phase::EvacuateRelease),
        (phase::Schedule::Global,    phase::Phase::EvacuateRelease),
        (phase::Schedule::Collector, phase::Phase::EvacuateRelease),
    ], 0);

    pub static ref NURSERY_COLLECTION: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Complex, plan::INIT_PHASE.clone()),
        (phase::Schedule::Global,  phase::Phase::CollectionSetSelection),
        // Prepare
        (phase::Schedule::Mutator,   phase::Phase::EvacuatePrepare),
        (phase::Schedule::Global,    phase::Phase::EvacuatePrepare),
        (phase::Schedule::Collector, phase::Phase::EvacuatePrepare),
        // Roots
        (phase::Schedule::Complex,   plan::PREPARE_STACKS.clone()),
        (phase::Schedule::Collector, phase::Phase::StackRoots),
        (phase::Schedule::Global,    phase::Phase::StackRoots),
        (phase::Schedule::Collector, phase::Phase::Roots),
        (phase::Schedule::Global,    phase::Phase::Roots),
        (phase::Schedule::Mutator,   phase::Phase::RefineCards),
        (phase::Schedule::Global,    phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RemSetRoots),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        // Refs
        (phase::Schedule::Collector, phase::Phase::SoftRefs),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::WeakRefs),
        (phase::Schedule::Collector, phase::Phase::Finalizable),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::PhantomRefs),
        // Scan remsets again
        (phase::Schedule::Mutator,   phase::Phase::RefineCards),
        (phase::Schedule::Global,    phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RefineCards),
        (phase::Schedule::Collector, phase::Phase::RemSetRoots),
        (phase::Schedule::Global,    phase::Phase::EvacuateClosure),
        (phase::Schedule::Collector, phase::Phase::EvacuateClosure),
        // Release
        (phase::Schedule::Mutator,   phase::Phase::EvacuateRelease),
        (phase::Schedule::Global,    phase::Phase::EvacuateRelease),
        (phase::Schedule::Collector, phase::Phase::EvacuateRelease),
        super::validate::schedule_validation_phase(),
        // Complete
        (phase::Schedule::Mutator, phase::Phase::Complete),
        (phase::Schedule::Complex, plan::FINISH_PHASE.clone()),
    ], 0);

    pub static ref COLLECTION: phase::Phase = phase::Phase::Complex(vec![
        (phase::Schedule::Complex, plan::INIT_PHASE.clone()),
        (phase::Schedule::Complex, ROOT_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, REF_TYPE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Complex, plan::COMPLETE_CLOSURE_PHASE.clone()),
        (phase::Schedule::Global,  phase::Phase::CollectionSetSelection),
        (phase::Schedule::Complex, {
            if !super::ENABLE_REMEMBERED_SETS {
                FULL_TRACE_EVACUATE_PHASE.clone()
            } else {
                REMSET_EVACUATE_PHASE.clone()
            }
        }),
        super::validate::schedule_validation_phase(),
        (phase::Schedule::Mutator, phase::Phase::Complete),
        (phase::Schedule::Complex, plan::FINISH_PHASE.clone()),
    ], 0);
}

pub fn get_collection_phase() -> phase::Phase {
    if super::ENABLE_GENERATIONAL_GC {
        if super::PLAN.gc_kind == super::g1::GCKind::Young {
            NURSERY_COLLECTION.clone()
        } else {
            COLLECTION.clone()
        }
    } else {
        COLLECTION.clone()
    }
}