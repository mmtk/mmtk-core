use crate::plan::Plan;
use crate::policy::immix::block::{Block, BlockState};
use crate::policy::space::Space;
use crate::scheduler::gc_work::*;
use crate::util::metadata::side_metadata::SideMetadataSpec;
use crate::util::ObjectReference;
use crate::vm::slot::Slot;
use crate::vm::*;
use crate::MMTK;
use crate::{scheduler::*, ObjectQueue};
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU8, Ordering};

#[allow(dead_code)]
pub struct SanityChecker<SL: Slot> {
    /// Visited objects
    refs: HashSet<ObjectReference>,
    /// Cached root edges for sanity root scanning
    root_slots: Vec<(Vec<SL>, RootKind)>,
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
    pub fn add_root_slots(&mut self, roots: Vec<SL>, kind: RootKind) {
        self.root_slots.push((roots, kind))
    }

    pub fn add_root_nodes(&mut self, roots: Vec<ObjectReference>) {
        self.root_nodes.push(roots)
    }

    /// Reset roots cache at the end of the sanity gc.
    pub(crate) fn clear_roots_cache(&mut self) {
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
            for (roots, kind) in &sanity_checker.root_slots {
                let mut w = SanityGCProcessEdges::<P::VM>::new(
                    roots.clone(),
                    true,
                    mmtk,
                    WorkBucketStage::Closure,
                );
                w.root_kind = Some(*kind);
                scheduler.work_buckets[WorkBucketStage::Closure].add(w);
            }
            for roots in &sanity_checker.root_nodes {
                scheduler.work_buckets[WorkBucketStage::Closure].add(ProcessRootNode::<
                    P::VM,
                    SanityGCProcessEdges<P::VM>,
                    SanityGCProcessEdges<P::VM>,
                >::new(
                    roots.clone(),
                    false,
                    false,
                    false,
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

static MARK_STATE: AtomicU8 = AtomicU8::new(0);
const MARK_BITS: SideMetadataSpec =
    crate::util::metadata::side_metadata::spec_defs::SANITY_MARK_BITS;

pub struct SanityPrepare<P: Plan> {
    pub plan: &'static P,
}

impl<P: Plan> SanityPrepare<P> {
    pub fn new(plan: &'static P) -> Self {
        Self { plan }
    }

    fn update_mark_state() {
        let mut mark_state = MARK_STATE.load(Ordering::SeqCst);
        if mark_state == 0 || mark_state == 255 {
            mark_state = 1;
        } else {
            mark_state += 1;
        }
        MARK_STATE.store(mark_state, Ordering::SeqCst);
    }
}

impl<P: Plan> GCWork<P::VM> for SanityPrepare<P> {
    fn do_work(&mut self, _worker: &mut GCWorker<P::VM>, mmtk: &'static MMTK<P::VM>) {
        Self::update_mark_state();
        <P::VM as VMBinding>::VMCollection::clear_cld_claimed_marks();
        info!("Sanity GC prepare");
        {
            let mut sanity_checker = mmtk.sanity_checker.lock().unwrap();
            sanity_checker.refs.clear();
        }
        crate::SANITY_LIVE_SIZE_IX.store(0, Ordering::Relaxed);
        crate::SANITY_LIVE_SIZE_LOS.store(0, Ordering::Relaxed);
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

// #[derive(Default)]
pub struct SanityGCProcessEdges<VM: VMBinding> {
    base: ProcessEdgesBase<VM>,
    edge: Option<VM::VMEdge>,
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

impl<VM: VMBinding> SanityGCProcessEdges<VM> {
    fn attempt_mark(&self, o: ObjectReference) -> bool {
        let mark_state = MARK_STATE.load(Ordering::SeqCst);
        loop {
            let old_value = MARK_BITS.load_atomic::<u8>(o.to_raw_address(), Ordering::SeqCst);
            if old_value == mark_state {
                return false;
            }
            if MARK_BITS
                .compare_exchange_atomic::<u8>(
                    o.to_raw_address(),
                    old_value,
                    mark_state,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return true;
            }
        }
    }
}

impl<VM: VMBinding> ProcessEdgesWork for SanityGCProcessEdges<VM> {
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;

    const OVERWRITE_REFERENCE: bool = false;
    fn new(
        slots: Vec<SlotOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        Self {
            base: ProcessEdgesBase::new(slots, roots, mmtk, bucket),
            // ..Default::default()
            edge: None,
        }
    }

    fn process_edge(&mut self, slot: EdgeOf<Self>) {
        let object = slot.load();
        self.edge = Some(slot);
        let new_object = self.trace_object(object);
        if Self::OVERWRITE_REFERENCE {
            slot.store(new_object);
        }
    }

    #[cfg(feature = "fragmentation_analysis")]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        use crate::util::address::{CLDScanPolicy, RefScanPolicy};
        if self.attempt_mark(object) {
            let lxr = self
                .mmtk()
                .get_plan()
                .downcast_ref::<crate::plan::lxr::LXR<VM>>()
                .unwrap();
            if lxr.immix_space.in_space(object) {
                crate::SANITY_LIVE_SIZE_IX.fetch_add(object.get_size::<VM>(), Ordering::Relaxed);
            } else {
                crate::SANITY_LIVE_SIZE_LOS.fetch_add(object.get_size::<VM>(), Ordering::Relaxed);
            }
            self.nodes.enqueue(object);
        }
        object
    }
    #[cfg(not(feature = "fragmentation_analysis"))]
    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        // gc_log!(
        //     "S {:?} -> {:?} r={} kind={:?}",
        //     self.edge,
        //     object,
        //     self.roots,
        //     self.root_kind
        // );
        if let Some(_lxr) = self
            .mmtk()
            .get_plan()
            .downcast_ref::<crate::plan::lxr::LXR<VM>>()
        {
            if self.edge.unwrap().to_address().is_mapped() {
                assert!(
                    !self.edge.unwrap().to_address().is_field_logged::<VM>(),
                    "{:?} -> {:?} is logged",
                    self.edge,
                    object
                );
            }
        }
        if object.is_null() {
            return object;
        }
        if self.attempt_mark(object) {
            // FIXME steveb consider VM-specific integrity check on reference.
            assert!(object.is_sane(), "Invalid reference {:?}", object);

            // Let plan check object
            assert!(
                object.to_raw_address().is_mapped(),
                "Invalid reference {:?} -> {:?}",
                self.edge,
                object
            );
            assert!(
                object.is_sane(),
                "Invalid reference {:?} -> {:?}",
                self.edge,
                object
            );
            if let Some(lxr) = self
                .mmtk()
                .get_plan()
                .downcast_ref::<crate::plan::lxr::LXR<VM>>()
            {
                assert!(
                    unsafe { object.to_raw_address().load::<usize>() } != 0xdead,
                    "{:?} -> {:?} is killed by decs",
                    self.edge,
                    object
                );
                assert!(
                    lxr.rc.count(object) > 0,
                    "{:?} -> {:?} has zero rc count",
                    self.edge,
                    object
                );
                assert!(
                    !crate::util::object_forwarding::is_forwarded_or_being_forwarded::<VM>(object),
                    "{:?} -> {:?} is forwarded",
                    self.edge,
                    object
                );
                if lxr.immix_space.in_space(object) {
                    assert_ne!(
                        Block::containing(object).get_state(),
                        BlockState::Unallocated,
                        "{:?}->{:?} block is released",
                        self.edge,
                        object
                    )
                }
                if lxr.current_pause().unwrap() == crate::plan::immix::Pause::FinalMark
                    || lxr.current_pause().unwrap() == crate::plan::immix::Pause::Full
                {
                    if !lxr.is_marked(object) {
                        flush_logs!()
                    }
                    assert!(
                        lxr.is_marked(object),
                        "{:?} -> {:?} is not marked, roots={} kind={:?}",
                        self.edge,
                        object,
                        self.roots,
                        self.root_kind,
                    )
                }
            }
            self.nodes.enqueue(object);
        }

        // If the valid object (VO) bit metadata is enabled, all live objects should have the VO
        // bit set when sanity GC starts.
        #[cfg(feature = "vo_bit")]
        if !crate::util::metadata::vo_bit::is_vo_bit_set(object) {
            panic!("VO bit is not set: {}", object);
        }

        object
    }

    fn create_scan_work(&self, nodes: Vec<ObjectReference>) -> Self::ScanObjectsWorkType {
        let mut x = ScanObjects::<Self>::new(nodes, false, false, false, WorkBucketStage::Closure);
        x.discovery = false;
        x
    }
}
