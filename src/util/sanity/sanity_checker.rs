use crate::plan::global::CopyContext;
use crate::plan::Plan;
use crate::scheduler::gc_work::*;
use crate::scheduler::*;
use crate::util::{Address, ObjectReference};
use crate::vm::*;
use crate::MMTK;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;

#[allow(dead_code)]
pub struct SanityChecker {
    refs: HashSet<ObjectReference>,
}

impl Default for SanityChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl SanityChecker {
    pub fn new() -> Self {
        Self {
            refs: HashSet::new(),
        }
    }
}

#[derive(Default)]
pub struct ScheduleSanityGC<P: Plan, W: CopyContext + WorkerLocal>(PhantomData<(P, W)>);

impl<P: Plan, W: CopyContext + WorkerLocal> ScheduleSanityGC<P, W> {
    pub fn new() -> Self {
        ScheduleSanityGC(PhantomData)
    }
}

impl<VM: VMBinding, P: Plan<VM = VM>, W: CopyContext + WorkerLocal> GCWork<VM>
    for ScheduleSanityGC<P, W>
{
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let scheduler = worker.scheduler();
        let plan = &mmtk.plan;

        scheduler.reset_state();

        plan.base().inside_sanity.store(true, Ordering::SeqCst);
        // Stop & scan mutators (mutator scanning can happen before STW)
        for mutator in VM::VMActivePlan::mutators() {
            scheduler.work_buckets[WorkBucketStage::Prepare]
                .add(ScanStackRoot::<SanityGCProcessEdges<VM>>(mutator));
        }
        scheduler.work_buckets[WorkBucketStage::Prepare]
            .add(ScanVMSpecificRoots::<SanityGCProcessEdges<VM>>::new());
        // Prepare global/collectors/mutators
        worker.scheduler().work_buckets[WorkBucketStage::Prepare].add(SanityPrepare::<P, W>::new(
            plan.downcast_ref::<P>().unwrap(),
        ));
        // Release global/collectors/mutators
        worker.scheduler().work_buckets[WorkBucketStage::Release].add(SanityRelease::<P, W>::new(
            plan.downcast_ref::<P>().unwrap(),
        ));
    }
}

pub struct SanityPrepare<P: Plan, W: CopyContext + WorkerLocal> {
    pub plan: &'static P,
    _p: PhantomData<W>,
}

unsafe impl<P: Plan, W: CopyContext + WorkerLocal> Sync for SanityPrepare<P, W> {}

impl<P: Plan, W: CopyContext + WorkerLocal> SanityPrepare<P, W> {
    pub fn new(plan: &'static P) -> Self {
        Self {
            plan,
            _p: PhantomData,
        }
    }
}

impl<P: Plan, W: CopyContext + WorkerLocal> GCWork<P::VM> for SanityPrepare<P, W> {
    fn do_work(&mut self, _worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        mmtk.plan.enter_sanity();
        {
            let mut sanity_checker = mmtk.sanity_checker.lock().unwrap();
            sanity_checker.refs.clear();
        }
        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Prepare]
                .add(PrepareMutator::<P::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group().workers {
            w.local_work_bucket.add(PrepareCollector::<W>::new());
        }
    }
}

pub struct SanityRelease<P: Plan, W: CopyContext + WorkerLocal> {
    pub plan: &'static P,
    _p: PhantomData<W>,
}

unsafe impl<P: Plan, W: CopyContext + WorkerLocal> Sync for SanityRelease<P, W> {}

impl<P: Plan, W: CopyContext + WorkerLocal> SanityRelease<P, W> {
    pub fn new(plan: &'static P) -> Self {
        Self {
            plan,
            _p: PhantomData,
        }
    }
}

impl<P: Plan, W: CopyContext + WorkerLocal> GCWork<P::VM> for SanityRelease<P, W> {
    fn do_work(&mut self, _worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        mmtk.plan.leave_sanity();
        for mutator in <P::VM as VMBinding>::VMActivePlan::mutators() {
            mmtk.scheduler.work_buckets[WorkBucketStage::Release]
                .add(ReleaseMutator::<P::VM>::new(mutator));
        }
        for w in &mmtk.scheduler.worker_group().workers {
            w.local_work_bucket.add(ReleaseCollector::<W>::new());
        }
    }
}

// #[derive(Default)]
pub struct SanityGCProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<SanityGCProcessEdges<VM>>,
    // phantom: PhantomData<VM>,
}

impl<VM: VMBinding> Deref for SanityGCProcessEdges<VM> {
    type Target = ProcessEdgesBase<Self>;
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
    const OVERWRITE_REFERENCE: bool = false;
    fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
        Self {
            base: ProcessEdgesBase::new(edges, mmtk),
            // ..Default::default()
        }
    }

    #[inline]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if object.is_null() {
            return object;
        }
        let mut sanity_checker = self.mmtk().sanity_checker.lock().unwrap();
        if !sanity_checker.refs.contains(&object) {
            // FIXME steveb consider VM-specific integrity check on reference.
            if !object.is_sane() {
                panic!("Invalid reference {:?}", object);
            }
            // TODO: Remove this check
            use crate::util::side_metadata::*;
            assert_eq!(load_atomic(SideMetadataSpec {
                scope: SideMetadataScope::Global,
                offset: 0,
                log_num_of_bits: 0,
                log_min_obj_size: 3,
            }, object.to_address()), 0b0, "object = {:?}", object);
            // Object is not "marked"
            sanity_checker.refs.insert(object); // "Mark" it
            ProcessEdgesWork::process_node(self, object);
        }
        object
    }
}
