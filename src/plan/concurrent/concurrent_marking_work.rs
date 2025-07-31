use crate::plan::concurrent::immix::global::ConcurrentImmix;
use crate::plan::concurrent::Pause;
use crate::plan::VectorQueue;
use crate::policy::gc_work::PolicyTraceObject;
use crate::policy::immix::TRACE_KIND_FAST;
use crate::policy::space::Space;
use crate::scheduler::gc_work::{ScanObjects, SlotOf};
use crate::util::ObjectReference;
use crate::vm::slot::Slot;

use crate::Plan;
use crate::{
    plan::ObjectQueue,
    scheduler::{gc_work::ProcessEdgesBase, GCWork, GCWorker, ProcessEdgesWork, WorkBucketStage},
    vm::*,
    MMTK,
};
use atomic::Ordering;
use std::ops::{Deref, DerefMut};

pub struct ConcurrentTraceObjects<VM: VMBinding> {
    plan: &'static ConcurrentImmix<VM>,
    // objects to mark and scan
    objects: Option<Vec<ObjectReference>>,
    // recursively generated objects
    next_objects: VectorQueue<ObjectReference>,
    worker: *mut GCWorker<VM>,
}

impl<VM: VMBinding> ConcurrentTraceObjects<VM> {
    const SATB_BUFFER_SIZE: usize = 8192;

    pub fn new(objects: Vec<ObjectReference>, mmtk: &'static MMTK<VM>) -> Self {
        let plan = mmtk
            .get_plan()
            .downcast_ref::<ConcurrentImmix<VM>>()
            .unwrap();
        let old_value = crate::NUM_CONCURRENT_TRACING_PACKETS.fetch_add(1, Ordering::SeqCst);
        let new_value = old_value + 1;
        probe!(mmtk, num_concurrent_tracing_packets_change, new_value);

        Self {
            plan,
            objects: Some(objects),
            next_objects: VectorQueue::default(),
            worker: std::ptr::null_mut(),
        }
    }

    pub fn worker(&self) -> &'static mut GCWorker<VM> {
        debug_assert_ne!(self.worker, std::ptr::null_mut());
        unsafe { &mut *self.worker }
    }

    #[cold]
    fn flush(&mut self) {
        if !self.next_objects.is_empty() {
            let objects = self.next_objects.take();
            let worker = self.worker();
            let w = Self::new(objects, worker.mmtk);
            worker.add_work(WorkBucketStage::Unconstrained, w);
        }
    }

    fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
        if self.plan.immix_space.in_space(object) {
            self.plan
                .immix_space
                .trace_object::<Self, { TRACE_KIND_FAST }>(self, object, None, self.worker());
        } else {
            self.plan.common().get_los().trace_object(self, object);
        }
        object
    }

    fn trace_objects(&mut self, objects: &[ObjectReference]) {
        for o in objects.iter() {
            self.trace_object(*o);
        }
    }

    fn scan_and_enqueue(&mut self, object: ObjectReference) {
        object.iterate_fields::<VM, _>(|s| {
            let Some(t) = s.load() else {
                return;
            };

            self.next_objects.push(t);
            if self.next_objects.len() > Self::SATB_BUFFER_SIZE {
                self.flush();
            }
        });
    }
}

impl<VM: VMBinding> ObjectQueue for ConcurrentTraceObjects<VM> {
    fn enqueue(&mut self, object: ObjectReference) {
        debug_assert!(
            object.to_raw_address().is_mapped(),
            "Invalid obj {:?}: address is not mapped",
            object
        );
        self.scan_and_enqueue(object);
    }
}

unsafe impl<VM: VMBinding> Send for ConcurrentTraceObjects<VM> {}

impl<VM: VMBinding> GCWork<VM> for ConcurrentTraceObjects<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, _mmtk: &'static MMTK<VM>) {
        self.worker = worker;
        let mut num_objects = 0;
        let mut num_next_objects = 0;
        let mut iterations = 0;
        // mark objects
        if let Some(objects) = self.objects.take() {
            self.trace_objects(&objects);
            num_objects = objects.len();
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
                num_next_objects += next_objects.len();
                iterations += 1;
            }
        }
        probe!(
            mmtk,
            concurrent_trace_objects,
            num_objects,
            num_next_objects,
            iterations
        );
        self.flush();

        let old_value = crate::NUM_CONCURRENT_TRACING_PACKETS.fetch_sub(1, Ordering::SeqCst);
        let new_value = old_value - 1;
        probe!(mmtk, num_concurrent_tracing_packets_change, new_value);
    }
}

pub struct ProcessModBufSATB {
    nodes: Option<Vec<ObjectReference>>,
}

impl ProcessModBufSATB {
    pub fn new(nodes: Vec<ObjectReference>) -> Self {
        Self { nodes: Some(nodes) }
    }
}

impl<VM: VMBinding> GCWork<VM> for ProcessModBufSATB {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        let mut w = if let Some(nodes) = self.nodes.take() {
            if nodes.is_empty() {
                return;
            }

            ConcurrentTraceObjects::new(nodes, mmtk)
        } else {
            return;
        };
        GCWork::do_work(&mut w, worker, mmtk);
    }
}

pub struct ProcessRootSlots<VM: VMBinding> {
    base: ProcessEdgesBase<VM>,
}

impl<VM: VMBinding> ProcessEdgesWork for ProcessRootSlots<VM> {
    type VM = VM;
    type ScanObjectsWorkType = ScanObjects<Self>;
    const OVERWRITE_REFERENCE: bool = false;
    const SCAN_OBJECTS_IMMEDIATELY: bool = true;

    fn new(
        slots: Vec<SlotOf<Self>>,
        roots: bool,
        mmtk: &'static MMTK<VM>,
        bucket: WorkBucketStage,
    ) -> Self {
        debug_assert!(roots);
        let base = ProcessEdgesBase::new(slots, roots, mmtk, bucket);
        Self { base }
    }

    fn flush(&mut self) {}

    fn trace_object(&mut self, _object: ObjectReference) -> ObjectReference {
        unreachable!()
    }

    fn process_slots(&mut self) {
        let pause = self
            .base
            .plan()
            .downcast_ref::<ConcurrentImmix<VM>>()
            .unwrap()
            .current_pause()
            .unwrap();
        // No need to scan roots in the final mark
        if pause == Pause::FinalMark {
            return;
        }
        let mut root_objects = Vec::with_capacity(Self::CAPACITY);
        if !self.slots.is_empty() {
            let slots = std::mem::take(&mut self.slots);
            for slot in slots {
                if let Some(object) = slot.load() {
                    root_objects.push(object);
                    if root_objects.len() == Self::CAPACITY {
                        // create the packet
                        let worker = self.worker();
                        let mmtk = self.mmtk();
                        let w = ConcurrentTraceObjects::new(root_objects.clone(), mmtk);

                        match pause {
                            Pause::InitialMark => worker.scheduler().postpone(w),
                            _ => unreachable!(),
                        }

                        root_objects.clear();
                    }
                }
            }
            if !root_objects.is_empty() {
                let worker = self.worker();
                let w = ConcurrentTraceObjects::new(root_objects.clone(), self.mmtk());

                match pause {
                    Pause::InitialMark => worker.scheduler().postpone(w),
                    _ => unreachable!(),
                }
            }
        }
    }

    fn create_scan_work(&self, _nodes: Vec<ObjectReference>) -> Self::ScanObjectsWorkType {
        unimplemented!()
    }
}

impl<VM: VMBinding> Deref for ProcessRootSlots<VM> {
    type Target = ProcessEdgesBase<VM>;
    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl<VM: VMBinding> DerefMut for ProcessRootSlots<VM> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}
