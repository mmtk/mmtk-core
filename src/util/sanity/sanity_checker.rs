use crate::plan::Plan;
use crate::scheduler::GCWork;
use crate::scheduler::GCWorker;
use crate::scheduler::WorkBucketStage;
use crate::util::scanning_helper;
use crate::util::ObjectReference;
use crate::vm::slot::Slot;
use crate::vm::{ObjectModel, VMBinding};
use crate::MMTK;
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
        debug!("Added {} root slots", roots.len());
        self.root_slots.push(roots)
    }

    pub fn add_root_nodes(&mut self, roots: Vec<ObjectReference>) {
        debug!("Added {} root nodes", roots.len());
        self.root_nodes.push(roots)
    }

    /// Reset roots cache at the end of the sanity gc.
    fn clear_roots_cache(&mut self) {
        debug!("Cleared roots cache");
        self.root_slots.clear();
        self.root_nodes.clear();
    }
}

pub struct ScheduleSanityGC<P: Plan> {
    plan: &'static P,
}

impl<P: Plan> ScheduleSanityGC<P> {
    pub fn new(plan: &'static P) -> Self {
        ScheduleSanityGC { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for ScheduleSanityGC<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        let scheduler = worker.scheduler();

        scheduler.reset_state();

        // We are going to do sanity GC which will traverse the object graph again. Reset slot logger to clear recorded slots.
        #[cfg(feature = "extreme_assertions")]
        mmtk.slot_logger.reset();

        mmtk.sanity_begin(); // Stop & scan mutators (mutator scanning can happen before STW)

        // Prepare global/collectors/mutators
        worker.scheduler().work_buckets[WorkBucketStage::Prepare]
            .add(SanityPrepare::<P>::new(self.plan));
        // Do the transitive closure
        worker.scheduler().work_buckets[WorkBucketStage::Closure]
            .add(SanityClosure::<P>::new(self.plan));
        // Release global/collectors/mutators
        worker.scheduler().work_buckets[WorkBucketStage::Release]
            .add(SanityRelease::<P>::new(self.plan));
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

pub struct SanityClosure<P: Plan> {
    pub plan: &'static P,
}

impl<P: Plan> SanityClosure<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }
}

impl<P: Plan> GCWork<P::VM> for SanityClosure<P> {
    fn do_work(&mut self, worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        info!("Sanity GC closure");
        let mut sanity_checker = mmtk.sanity_checker.lock().unwrap();

        let mut queue = Vec::new();

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
        for roots in &sanity_checker.root_slots {
            queue.extend(roots.iter().flat_map(|slot| slot.load()));
        }
        for roots in &sanity_checker.root_nodes {
            queue.extend(roots);
        }

        let tls = worker.tls;

        while let Some(object) = queue.pop() {
            if !sanity_checker.refs.insert(object) {
                continue;
            }

            trace!("Doing sanity check on object {object}");

            // FIXME steveb consider VM-specific integrity check on reference.
            assert!(
                object.is_sane(),
                "`object.is_sane()` returned false.  object: {object}",
            );

            // Let plan check object
            assert!(
                self.plan.sanity_check_object(object),
                "plan.sanity_check_object(object) returned false. object: {object}",
            );

            // Let VM check object
            assert!(
                <P::VM as VMBinding>::VMObjectModel::is_object_sane(object),
                "VMObjectModel::is_object_sane(object) returned false. object: {object}",
            );

            // Enqueue children.  If a child is already visited, it will be skipped at the beginning
            // of the loop.
            scanning_helper::visit_children_non_moving::<P::VM>(tls, object, &mut |child| {
                queue.push(child);
                child
            });
        }
    }
}
