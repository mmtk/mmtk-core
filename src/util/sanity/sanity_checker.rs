use crate::plan::Plan;
use crate::scheduler::gc_work::*;
use crate::util::ObjectReference;
use crate::vm::edge_shape::Edge;
use crate::vm::*;
use crate::MMTK;
use crate::{scheduler::*, ObjectQueue};
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;

#[allow(dead_code)]
pub struct SanityChecker<ES: Edge> {
    /// Visited objects
    refs: HashSet<ObjectReference>,
    /// Cached root edges for sanity root scanning
    root_edges: Vec<Vec<ES>>,
    /// Cached root nodes for sanity root scanning
    root_nodes: Vec<Vec<ObjectReference>>,
}

impl<ES: Edge> Default for SanityChecker<ES> {
    fn default() -> Self {
        Self::new()
    }
}

impl<ES: Edge> SanityChecker<ES> {
    pub fn new() -> Self {
        Self {
            refs: HashSet::new(),
            root_edges: vec![],
            root_nodes: vec![],
        }
    }

    /// Cache a list of root edges to the sanity checker.
    pub fn add_root_edges(&mut self, roots: Vec<ES>) {
        self.root_edges.push(roots)
    }

    pub fn add_root_nodes(&mut self, roots: Vec<ObjectReference>) {
        self.root_nodes.push(roots)
    }

    /// Reset roots cache at the end of the sanity gc.
    fn clear_roots_cache(&mut self) {
        self.root_edges.clear();
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

        // We are going to do sanity GC which will traverse the object graph again. Reset edge logger to clear recorded edges.
        #[cfg(feature = "extreme_assertions")]
        mmtk.edge_logger.reset();

        plan.base().inside_sanity.store(true, Ordering::SeqCst);
        // Stop & scan mutators (mutator scanning can happen before STW)

        // We use the cached roots for sanity gc, based on the assumption that
        // the stack scanning triggered by the selected plan is correct and precise.
        // FIXME(Wenyu,Tianle): When working on eager stack scanning on OpenJDK,
        // the stack scanning may be broken. Uncomment the following lines to
        // collect the roots again.
        // Also, remember to call `DerivedPointerTable::update_pointers(); DerivedPointerTable::clear();`
        // in openjdk binding before the second round of roots scanning.
        // for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
        //     scheduler.work_buckets[WorkBucketStage::Prepare]
        //         .add(ScanStackRoot::<SanityGCProcessEdges<P::VM>>(mutator));
        // }
        {
            let sanity_checker = mmtk.sanity_checker.lock().unwrap();
            for roots in &sanity_checker.root_edges {
                scheduler.work_buckets[WorkBucketStage::Closure].add(
                    SanityGCProcessEdges::<P::VM>::new(roots.clone(), true, mmtk),
                );
            }
            for roots in &sanity_checker.root_nodes {
                scheduler.work_buckets[WorkBucketStage::Closure].add(ScanObjects::<
                    SanityGCProcessEdges<P::VM>,
                >::new(
                    roots.clone(), false, true
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
        mmtk.get_plan().enter_sanity();
        {
            let mut sanity_checker = mmtk.sanity_checker.lock().unwrap();
            sanity_checker.refs.clear();
        }
        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                .add(PrepareMutator::<P::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group.workers_shared {
            let result = w.designated_work.push(Box::new(PrepareCollector));
            debug_assert!(result.is_ok());
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
        mmtk.get_plan().leave_sanity();
        mmtk.sanity_checker.lock().unwrap().clear_roots_cache();
        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Release]
                .add(ReleaseMutator::<P::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group.workers_shared {
            let result = w.designated_work.push(Box::new(ReleaseCollector));
            debug_assert!(result.is_ok());
        }
    }
}

// #[derive(Default)]
pub struct SanityGCProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding> Deref for SanityGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for SanityGCProcessEdges<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl<VM: VMBinding> ProcessEdgesWork for SanityGCProcessEdges<VM> {
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;

    const OVERWRITE_REFERENCE: bool = false;
    fn new(edges: Vec<EdgeOf<Self>>, roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        Self {
            base: ProcessEdgesBase::new(edges, roots, mmtk),
            // ..Default::default()
        }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let mut sanity_checker = self.mmtk().sanity_checker.lock().unwrap();
        if !sanity_checker.refs.contains(&object) {
            // FIXME steveb consider VM-specific integrity check on reference.
            assert!(object.is_sane(), "Invalid reference {:?}", object);

            // Let plan check object
            assert!(
                self.mmtk().get_plan().sanity_check_object(object),
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
            self.nodes.enqueue(object);
        }

        // If the valid object (VO) bit metadata is enabled, all live objects should have the VO
        // bit set when sanity GC starts.
        #[cfg(feature = "vo_bit")]
        if !crate::util::metadata::vo_bit::is_vo_bit_set::<VM>(object) {
            panic!("VO bit is not set: {}", object);
        }

        object
    }

    fn create_scan_work(
        &self,
        nodes: Vec<ObjectReference>,
        roots: bool,
    ) -> Self::ScanObjectsWorkType {
        ScanObjects::<Self>::new(nodes, false, roots)
    }
}
