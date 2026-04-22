use super::super::LXR;
use crate::plan::concurrent::Pause;
use crate::plan::VectorQueue;
use crate::policy::immix::block::Block;
use crate::policy::space::Space;
use crate::scheduler::gc_work::{ScanObjects, SlotOf};
use crate::scheduler::RootKind;
use crate::util::copy::CopySemantics;
use crate::util::rc::RefCountHelper;
use crate::util::ObjectReference;
use crate::vm::slot::Slot;
use crate::{
    plan::ObjectQueue,
    scheduler::{gc_work::ProcessEdgesBase, GCWork, GCWorker, ProcessEdgesWork, WorkBucketStage},
    vm::*,
    MMTK,
};
use atomic::Ordering;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

pub struct LXRConcurrentTraceObjects<VM: VMBinding> {
    plan: &'static LXR<VM>,
    // objects to mark and scan
    objects: Option<Vec<ObjectReference>>,
    objects_arc: Option<Arc<Vec<ObjectReference>>>,
    // recursively generated objects
    next_objects: VectorQueue<ObjectReference>,
    rc: RefCountHelper<VM>,
    worker: *mut GCWorker<VM>,
}

impl<VM: VMBinding> LXRConcurrentTraceObjects<VM> {
    const SATB_BUFFER_SIZE: usize = 8192;

    pub fn new(objects: Vec<ObjectReference>, mmtk: &'static MMTK<VM>) -> Self {
        let plan = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        super::super::NUM_CONCURRENT_TRACING_PACKETS.fetch_add(1, Ordering::SeqCst);
        Self {
            plan,
            objects: Some(objects),
            objects_arc: None,
            next_objects: VectorQueue::default(),
            rc: RefCountHelper::NEW,
            worker: std::ptr::null_mut(),
        }
    }

    pub fn new_arc(objects: Arc<Vec<ObjectReference>>, mmtk: &'static MMTK<VM>) -> Self {
        let plan = mmtk.get_plan().downcast_ref::<LXR<VM>>().unwrap();
        super::super::NUM_CONCURRENT_TRACING_PACKETS.fetch_add(1, Ordering::SeqCst);
        Self {
            plan,
            objects: None,
            objects_arc: Some(objects),
            next_objects: VectorQueue::default(),
            rc: RefCountHelper::NEW,
            worker: std::ptr::null_mut(),
        }
    }

    #[cold]
    fn flush(&mut self) {
        if !self.next_objects.is_empty() {
            let objects = self.next_objects.take();
            let worker = GCWorker::<VM>::current();
            debug_assert!(self.plan.cm_enabled());
            let w = Self::new(objects, worker.mmtk);
            worker.add_work(WorkBucketStage::ConcurrentResumable, w);
        }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if self.rc.count(object) == 0 {
            return object;
        }
        if self.plan.immix_space.in_space(object) {
            self.plan
                .immix_space
                .trace_object_without_moving_rc(self, object);
        } else {
            self.plan.los().trace_object(self, object);
        }
        object
    }

    fn trace_objects(&mut self, objects: &[ObjectReference]) {
        for o in objects {
            self.trace_object(*o);
        }
    }

    fn scan_and_enqueue<const CHECK_REMSET: bool>(&mut self, object: ObjectReference) {
        object.iterate_fields::<VM, _>(|s| {
            let Some(t) = s.load() else {
                return;
            };
            if super::super::MATURE_EVACUATION && CHECK_REMSET && self.plan.in_defrag(t) {
                self.plan.mature_evac_remset.record(s, t, self.plan);
            }
            self.next_objects.push(t);
            if self.next_objects.len() > Self::SATB_BUFFER_SIZE {
                self.flush();
            }
        });
    }
}

impl<VM: VMBinding> ObjectQueue for LXRConcurrentTraceObjects<VM> {
    fn enqueue(&mut self, object: ObjectReference) {
        if cfg!(feature = "sanity") {
            assert!(
                object.to_raw_address().is_mapped(),
                "Invalid obj {:?}: address is not mapped",
                object
            );
        }
        let should_check_remset = !self.plan.in_defrag(object);
        if should_check_remset {
            self.scan_and_enqueue::<true>(object)
        } else {
            self.scan_and_enqueue::<false>(object)
        }
    }
}

unsafe impl<VM: VMBinding> Send for LXRConcurrentTraceObjects<VM> {}

impl<VM: VMBinding> GCWork<VM> for LXRConcurrentTraceObjects<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        self.worker = worker;
        debug_assert!(!mmtk.scheduler.work_buckets[WorkBucketStage::Initial].is_open());
        // mark objects
        if let Some(objects) = self.objects.take() {
            self.trace_objects(&objects)
        } else if let Some(objects) = self.objects_arc.take() {
            self.trace_objects(&objects)
        }
        let pause_opt = self.plan.current_pause();
        if pause_opt == Some(Pause::FinalMark) || pause_opt.is_none() {
            let mut next_objects = vec![];
            while !self.next_objects.is_empty() {
                let pause_opt = self.plan.current_pause();
                if !(pause_opt == Some(Pause::FinalMark) || pause_opt.is_none()) {
                    break;
                }
                next_objects.clear();
                self.next_objects.swap(&mut next_objects);
                self.trace_objects(&next_objects);
            }
        }
        self.flush();
        // CM: Decrease counter
        super::super::NUM_CONCURRENT_TRACING_PACKETS.fetch_sub(1, Ordering::SeqCst);
        debug_assert!(!mmtk.scheduler.work_buckets[WorkBucketStage::Initial].is_open());
    }
}

pub struct ProcessModBufSATB {
    nodes: Option<Vec<ObjectReference>>,
    nodes_arc: Option<Arc<Vec<ObjectReference>>>,
}

impl ProcessModBufSATB {
    pub fn new(nodes: Vec<ObjectReference>) -> Self {
        // super::NUM_CONCURRENT_TRACING_PACKETS.fetch_add(1, Ordering::SeqCst);
        Self {
            nodes: Some(nodes),
            nodes_arc: None,
        }
    }
    pub fn new_arc(nodes: Arc<Vec<ObjectReference>>) -> Self {
        // super::NUM_CONCURRENT_TRACING_PACKETS.fetch_add(1, Ordering::SeqCst);
        Self {
            nodes: None,
            nodes_arc: Some(nodes),
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for ProcessModBufSATB {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let mut w = if let Some(nodes) = self.nodes.take() {
            if nodes.is_empty() {
                return;
            }
            if cfg!(any(feature = "sanity", debug_assertions)) {
                for o in &nodes {
                    assert!(
                        o.to_raw_address().is_mapped(),
                        "Invalid object {:?}: address is not mapped",
                        o
                    );
                }
            }
            LXRConcurrentTraceObjects::new(nodes, mmtk)
        } else if let Some(nodes) = self.nodes_arc.take() {
            if nodes.is_empty() {
                return;
            }
            if cfg!(any(feature = "sanity", debug_assertions)) {
                for o in &*nodes {
                    assert!(
                        o.to_raw_address().is_mapped(),
                        "Invalid object {:?}: address is not mapped",
                        o
                    );
                }
            }
            LXRConcurrentTraceObjects::new_arc(nodes, mmtk)
        } else {
            return;
        };

        let current_pause = mmtk
            .get_plan()
            .downcast_ref::<LXR<VM>>()
            .unwrap()
            .current_pause();
        if current_pause != Some(Pause::FinalMark) {
            worker.scheduler().work_buckets[WorkBucketStage::ConcurrentResumable].add(w);
        } else {
            GCWork::do_work(&mut w, worker, mmtk);
        }
    }
}

pub struct LXRStopTheWorldProcessEdges<VM: VMBinding, const FULL_GC: bool> {
    lxr: &'static LXR<VM>,
    pause: Pause,
    base: ProcessEdgesBase<VM>,
    forwarded_roots: Vec<ObjectReference>,
    next_slots: VectorQueue<SlotOf<Self>>,
    next_slot_count: u32,
    remset_recorded_slots: bool,
    should_record_forwarded_roots: bool,
}

impl<VM: VMBinding, const FULL_GC: bool> LXRStopTheWorldProcessEdges<VM, FULL_GC> {
    pub(super) fn new_remset(slots: Vec<SlotOf<Self>>, mmtk: &'static MMTK<VM>) -> Self {
        let mut me = Self::new(slots, false, mmtk, WorkBucketStage::Closure);
        me.remset_recorded_slots = true;
        me
    }
}

impl<VM: VMBinding, const FULL_GC: bool> ProcessEdgesWork
    for LXRStopTheWorldProcessEdges<VM, FULL_GC>
{
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;
    const OVERWRITE_REFERENCE: bool = super::super::MATURE_EVACUATION;

    fn new(
        slots: Vec<SlotOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        let base = ProcessEdgesBase::new(slots, roots, mmtk, bucket);
        let lxr = base.plan().downcast_ref::<LXR<VM>>().unwrap();
        Self {
            lxr,
            base,
            pause: Pause::RefCount,
            forwarded_roots: vec![],
            next_slots: VectorQueue::new(),
            next_slot_count: 0,
            remset_recorded_slots: false,
            should_record_forwarded_roots: false,
        }
    }

    #[cold]
    fn flush(&mut self) {
        if !self.next_slots.is_empty() {
            let slots = self.next_slots.take();
            let w = Self::new(slots, false, self.mmtk(), self.bucket);
            self.worker()
                .add_boxed_work(WorkBucketStage::Unconstrained, Box::new(w));
        }
        assert!(self.nodes.is_empty());
        self.next_slot_count = 0;
    }

    /// Trace  and evacuate objects.
    fn trace_object(&mut self, _object: ObjectReference) -> ObjectReference {
        unreachable!()
    }

    fn process_slots(&mut self) {
        self.should_record_forwarded_roots = self.roots
            && !self
                .root_kind
                .map(|r| r.should_skip_decs())
                .unwrap_or_default();
        self.pause = self.lxr.current_pause().unwrap();
        if self.should_record_forwarded_roots {
            self.forwarded_roots.reserve(self.slots.len());
        }
        let slots = std::mem::take(&mut self.slots);
        if self.roots && self.root_kind == Some(RootKind::Weak) {
            self.process_slots_impl::<true, false>(&slots);
        } else if self.remset_recorded_slots {
            self.process_slots_impl::<false, true>(&slots);
        } else {
            self.process_slots_impl::<false, false>(&slots);
        }
        self.roots = false;
        self.remset_recorded_slots = false;
        let should_record_forwarded_roots = self.should_record_forwarded_roots;
        self.should_record_forwarded_roots = false;
        let mut slots = vec![];
        while !self.next_slots.is_empty() {
            self.next_slot_count = 0;
            slots.clear();
            self.next_slots.swap(&mut slots);
            self.process_slots_impl::<false, false>(&slots);
        }
        self.flush();
        if should_record_forwarded_roots {
            let roots = std::mem::take(&mut self.forwarded_roots);
            self.lxr.curr_roots.read().unwrap().push(roots);
        }
    }

    fn process_slot(&mut self, _slot: SlotOf<Self>) {
        unreachable!()
    }

    fn create_scan_work(&self, _nodes: Vec<ObjectReference>) -> ScanObjects<Self> {
        unreachable!()
    }
}

impl<VM: VMBinding, const FULL_GC: bool> LXRStopTheWorldProcessEdges<VM, FULL_GC> {
    #[inline]
    fn full_gc_trace_object<const WEAK_ROOT: bool>(
        &mut self,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(FULL_GC);
        debug_assert!(object.is_in_any_space());
        debug_assert!(object.to_raw_address().is_aligned_to(8));
        // debug_assert!(object.class_is_valid::<VM>());
        if WEAK_ROOT && !Block::containing(object).is_defrag_source() {
            return object;
        }
        let x = if self.lxr.immix_space.in_space(object) {
            let pause = self.pause;
            let worker = self.worker();
            self.lxr.immix_space.rc_trace_object(
                self,
                object,
                CopySemantics::DefaultCopy,
                pause,
                true,
                worker,
            )
        } else {
            self.lxr.los().trace_object(self, object)
        };
        if self.should_record_forwarded_roots {
            self.forwarded_roots.push(x)
        }
        x
    }

    #[inline]
    fn mature_evac_trace_object<const WEAK_ROOT: bool, const REMSET: bool>(
        &mut self,
        object: ObjectReference,
    ) -> ObjectReference {
        debug_assert!(!FULL_GC);
        // The memory (lines) of these slots can be reused at any time during mature evacuation.
        // Filter out invalid target objects.
        if REMSET && (!object.is_in_any_space() || !object.to_raw_address().is_aligned_to(8)) {
            return object;
        }
        if self.lxr.rc.count(object) == 0 {
            return object;
        }
        if WEAK_ROOT && !Block::containing(object).is_defrag_source() {
            return object;
        }
        debug_assert!(object.is_in_any_space(), "Invalid {:?}", object);
        debug_assert!(
            object.to_raw_address().is_aligned_to(8),
            "Invalid {:?} remset={}",
            object,
            self.remset_recorded_slots
        );
        let object = object.get_forwarded_object().unwrap_or(object);
        let new_object = if self.lxr.immix_space.in_space(object) {
            if self
                .lxr
                .rc
                .address_is_in_straddle_line(object.to_raw_address())
            {
                return object;
            }
            let pause = self.pause;
            let worker = self.worker();
            self.lxr.immix_space.rc_trace_object(
                self,
                object,
                CopySemantics::DefaultCopy,
                pause,
                true,
                worker,
            )
        } else {
            self.lxr.los().trace_object(self, object)
        };
        if self.should_record_forwarded_roots {
            self.forwarded_roots.push(new_object)
        }
        new_object
    }

    #[inline]
    fn __process_slot<const WEAK_ROOT: bool, const REMSET: bool>(&mut self, slot: SlotOf<Self>) {
        let Some(object) = slot.load() else {
            return;
        };
        let new_object = if !FULL_GC {
            self.mature_evac_trace_object::<WEAK_ROOT, REMSET>(object)
        } else {
            self.full_gc_trace_object::<WEAK_ROOT>(object)
        };
        if Self::OVERWRITE_REFERENCE && new_object != object {
            slot.store(new_object);
        }
    }

    fn process_slots_impl<const WEAK_ROOT: bool, const REMSET: bool>(
        &mut self,
        slots: &[VM::VMSlot],
    ) {
        for s in slots {
            self.__process_slot::<WEAK_ROOT, REMSET>(*s);
        }
    }
}

impl<VM: VMBinding, const FULL_GC: bool> ObjectQueue for LXRStopTheWorldProcessEdges<VM, FULL_GC> {
    fn enqueue(&mut self, object: ObjectReference) {
        let limit: usize = if FULL_GC { 8192 } else { 1024 };
        object.iterate_fields::<VM, _>(|s| {
            let Some(o) = s.load() else {
                return;
            };
            if self.lxr.is_marked(o) && !self.lxr.in_defrag(o) {
                return;
            }
            self.next_slots.push(s);
            self.next_slot_count += 1;
            if self.next_slot_count as usize >= limit {
                self.flush();
            }
        });
    }
}

impl<VM: VMBinding, const FULL_GC: bool> Deref for LXRStopTheWorldProcessEdges<VM, FULL_GC> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding, const FULL_GC: bool> DerefMut for LXRStopTheWorldProcessEdges<VM, FULL_GC> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
