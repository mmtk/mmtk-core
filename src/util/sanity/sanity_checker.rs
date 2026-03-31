use crate::plan::tracing::Trace;
use crate::plan::Plan;
use crate::scheduler::gc_work::TracingProcessPinningRoots;
use crate::scheduler::gc_work::TracingProcessSlots;
use crate::util::ObjectReference;
use crate::vm::slot::Slot;
use crate::vm::*;
use crate::MMTK;
use crate::{scheduler::*, ObjectQueue};
use std::collections::HashSet;

#[allow(dead_code)]
pub struct SanityChecker<SL: Slot> {
    /// Visited objects
    refs: HashSet<ObjectReference>,
    /// Cached root slots for sanity root scanning
    root_slots: Vec<Vec<SL>>,
    /// Cached root nodes for sanity root scanning
    root_nodes: Vec<Vec<ObjectReference>>,
}

impl<SL: Slot> Default for SanityChecker<SL> {
    fn default() -> Self {
        Self::new()
    }
}

impl<SL: Slot> SanityChecker<SL> {
    pub fn new() -> Self {
        Self {
            refs: HashSet::new(),
            root_slots: vec![],
            root_nodes: vec![],
        }
    }

    /// Cache a list of root slots to the sanity checker.
    pub fn add_root_slots(&mut self, roots: Vec<SL>) {
        self.root_slots.push(roots)
    }

    pub fn add_root_nodes(&mut self, roots: Vec<ObjectReference>) {
        self.root_nodes.push(roots)
    }

    /// Reset roots cache at the end of the sanity gc.
    fn clear_roots_cache(&mut self) {
        self.root_slots.clear();
        self.root_nodes.clear();
    }
}

pub struct ScheduleSanityGC<P: Plan> {
    _plan: &'static P,
}

impl<P: Plan> ScheduleSanityGC<P> {
    pub fn new(plan: &'static P) -> Self {
        ScheduleSanityGC { _plan: plan }
    }
}

impl<P: Plan> GCWork<P::VM> for ScheduleSanityGC<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        let scheduler = worker.scheduler();
        let plan = mmtk.get_plan();

        scheduler.reset_state();

        // We are going to do sanity GC which will traverse the object graph again. Reset slot logger to clear recorded slots.
        #[cfg(feature = "extreme_assertions")]
        mmtk.slot_logger.reset();

        mmtk.sanity_begin(); // Stop & scan mutators (mutator scanning can happen before STW)

        // We use the cached roots for sanity gc, based on the assumption that
        // the stack scanning triggered by the selected plan is correct and precise.
        // FIXME(Wenyu,Tianle): When working on eager stack scanning on OpenJDK,
        // the stack scanning may be broken. Uncomment the following lines to
        // collect the roots again.
        // Also, remember to call `DerivedPointerTable::update_pointers(); DerivedPointerTable::clear();`
        // in openjdk binding before the second round of roots scanning.
        // for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
        //     scheduler.work_buckets[WorkBucketStage::Prepare]
        //         .add(ScanMutatorRoots::<SanityGCProcessEdges<P::VM>>(mutator));
        // }
        {
            let sanity_checker = mmtk.sanity_checker.lock().unwrap();
            for roots in &sanity_checker.root_slots {
                scheduler.work_buckets[WorkBucketStage::Closure].add(TracingProcessSlots::new(
                    SanityTrace::from_mmtk(mmtk),
                    roots.clone(),
                    true,
                    WorkBucketStage::Closure,
                ));
            }
            for roots in &sanity_checker.root_nodes {
                scheduler.work_buckets[WorkBucketStage::Closure].add(TracingProcessPinningRoots::<
                    P::VM,
                    SanityTrace<P::VM>,
                    SanityTrace<P::VM>,
                >::new(
                    roots.clone(),
                    WorkBucketStage::Closure,
                ));
            }
        }
        // Prepare global/collectors/mutators
        worker.scheduler().work_buckets[WorkBucketStage::Prepare]
            .add(SanityPrepare::<P>::new(plan.downcast_ref::<P>().unwrap()));
        // Release global/collectors/mutators
        worker.scheduler().work_buckets[WorkBucketStage::Release]
            .add(SanityRelease::<P>::new(plan.downcast_ref::<P>().unwrap()));
    }
}

pub struct SanityPrepare<P: Plan> {
    pub plan: &'static P,
}

impl<P: Plan> SanityPrepare<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for SanityPrepare<P> {
    fn do_work(&mut self, _worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        info!("Sanity GC prepare");
        {
            let mut sanity_checker = mmtk.sanity_checker.lock().unwrap();
            sanity_checker.refs.clear();
        }
    }
}

pub struct SanityRelease<P: Plan> {
    pub plan: &'static P,
}

impl<P: Plan> SanityRelease<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for SanityRelease<P> {
    fn do_work(&mut self, _worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        info!("Sanity GC release");
        mmtk.sanity_checker.lock().unwrap().clear_roots_cache();
        mmtk.sanity_end();
    }
}

pub struct SanityTrace<VM: VMBinding> {
    mmtk: &'static MMTK<VM>,
}

impl<VM: VMBinding> Clone for SanityTrace<VM> {
    fn clone(&self) -> Self {
        Self { mmtk: self.mmtk }
    }
}

impl<VM: VMBinding> Trace for SanityTrace<VM> {
    type VM = VM;

    fn from_mmtk(mmtk: &'static MMTK<Self::VM>) -> Self {
        Self { mmtk }
    }

    fn trace_object<Q: ObjectQueue>(
        &mut self,
        _worker: &mut GCWorker<Self::VM>,
        object: ObjectReference,
        queue: &mut Q,
    ) -> ObjectReference {
        // TODO: Rewrite the sanity checker as a simple loop in a single work packet.
        // It is very inefficient to acquire the mutex in every single `trace_object`.
        let mut sanity_checker = self.mmtk.sanity_checker.lock().unwrap();
        if !sanity_checker.refs.contains(&object) {
            // FIXME steveb consider VM-specific integrity check on reference.
            assert!(object.is_sane(), "Invalid reference {:?}", object);

            // Let plan check object
            assert!(
                self.mmtk.get_plan().sanity_check_object(object),
                "Invalid reference {:?}",
                object
            );

            // Let VM check object
            assert!(
                VM::VMObjectModel::is_object_sane(object),
                "Invalid reference {:?}",
                object
            );

            // Object is not "marked"
            sanity_checker.refs.insert(object); // "Mark" it
            trace!("Sanity mark object {}", object);
            queue.enqueue(object);
        }

        // If the valid object (VO) bit metadata is enabled, all live objects should have the VO
        // bit set when sanity GC starts.
        #[cfg(feature = "vo_bit")]
        if !crate::util::metadata::vo_bit::is_vo_bit_set(object) {
            panic!("VO bit is not set: {}", object);
        }

        object
    }

    fn post_scan_object(&mut self, _object: ObjectReference) {
        // Do nothing
    }

    fn may_move_objects() -> bool {
        false
    }
}
