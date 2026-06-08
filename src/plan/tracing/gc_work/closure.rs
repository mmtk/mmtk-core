use std::{collections::VecDeque, marker::PhantomData};

use crate::{
    plan::{
        tracing::{gc_work::DefaultObjectTracerContext, SlotOfTrace, Trace},
        VectorQueue,
    },
    scheduler::{GCWork, GCWorker, GCWorkerShared, WorkBucketStage, EDGES_WORK_BUFFER_SIZE},
    util::{ObjectReference, VMWorkerThread},
    vm::{slot::Slot, ObjectTracerContext, Scanning, VMBinding},
    MMTK,
};

/// A work packet for processing slots during a stop-the-world tracing GC and the final mark pause
/// of a concurrent GC.
///
/// It will call `trace_object` on the value of each slot, and updates the slot if the object is
/// moved or forwarded.  It will spawn or immediately run the [`ProcessNodes`] work packet to
/// scan newly traced objects.
pub struct ProcessSlots<T: Trace> {
    slot_queue: VecDeque<SlotOfTrace<T>>,
    bucket: WorkBucketStage,
}

impl<T: Trace> ProcessSlots<T> {
    /// The nominative capacity of the deque.
    ///
    /// It will try to split the work packet if the deque has too many elements.
    const CAPACITY: usize = EDGES_WORK_BUFFER_SIZE;

    /// The number of slots to push before it proactively splits the work packet.
    ///
    /// The purpose of this is preventing a pathological case where one worker keeps popping and
    /// pushing elements without reaching the capacity.
    const PUSHES_BEFORE_SPLIT: usize = Self::CAPACITY;

    pub fn new(slots: Vec<SlotOfTrace<T>>, bucket: WorkBucketStage) -> Self {
        let mut slot_queue = VecDeque::with_capacity(Self::CAPACITY);
        slot_queue.extend(slots);
        Self::from_deque(slot_queue, bucket)
    }

    pub fn from_deque(slot_queue: VecDeque<SlotOfTrace<T>>, bucket: WorkBucketStage) -> Self {
        Self { slot_queue, bucket }
    }

    fn split(&mut self, worker: &mut GCWorker<T::VM>) {
        debug_assert!(!self.slot_queue.is_empty());

        let parked_workers_sloppy = worker.scheduler().worker_monitor.parked_workers_sloppy();

        // Divide the current work packet into two parts by default, but add one more part for every
        // idle worker.
        let divider = 2 + parked_workers_sloppy;

        // Each part has at least one element.
        let split_len = (self.slot_queue.len() / divider).max(1);

        let mut remaining_parts = divider - 1; // Leave one part for myself.
        while remaining_parts > 0 && self.slot_queue.len() > split_len {
            let split_queue = self.slot_queue.split_off(split_len);
            let work = Self::from_deque(split_queue, self.bucket);
            worker.add_work(self.bucket, work);

            remaining_parts -= 1;
        }
    }
}

impl<T: Trace> GCWork<T::VM> for ProcessSlots<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        let tls = worker.tls;
        let trace = T::from_mmtk(mmtk);

        // The number of slots at the beginning of the work packet.
        let num_initial_slots = self.slot_queue.len();
        // The number of slots visited.
        let mut num_visited_slots = 0;
        // The number of slots enqueued.
        let mut num_enqueued_slots = 0;
        // The number of empty slots visited
        let mut num_empty_slots = 0;
        // The number of objects scanned.
        let mut num_scanned_objects = 0;
        // The number of times we split the work packet.
        let mut num_splits = 0;

        // The number of slots pushed since we create or split the work packet.
        let mut num_pushes = 0;

        while let Some(slot) = self.slot_queue.pop_back() {
            num_visited_slots += 1;

            #[cfg(feature = "extreme_assertions")]
            if crate::util::slot_logger::should_check_duplicate_slots(mmtk.get_plan()) {
                // log slot, panic if already logged
                mmtk.slot_logger.log_slot(*slot);
            }

            // Skip empty slot.
            let Some(target) = slot.load() else {
                num_empty_slots += 1;
                continue;
            };

            // trace.trace_object borrows the `worker`, preventing us from flushing the queue inside
            // (which also requires the worker).  We store the reference to a local variable, first.
            let mut enqueued_object = None;
            let new_target = trace.trace_object(worker, target, &mut enqueued_object);

            // Store back to the slot if the object is moved.
            if T::may_move_objects() && new_target != target {
                slot.store(new_target);
            }

            // Scan the object if it is enqueued.
            if let Some(enqueued_object) = enqueued_object {
                num_scanned_objects += 1;

                // If there is any object that does not support slot enqueuing, the VM binding
                // should choose `ProcessNodes` as the default tracing strategy.
                debug_assert!(
                    <T::VM as VMBinding>::VMScanning::support_slot_enqueuing(tls, enqueued_object),
                    "Object {enqueued_object} does not support slot enqueuing."
                );

                trace!("Scan object {} for slots...", enqueued_object);
                <T::VM as VMBinding>::VMScanning::scan_object(tls, enqueued_object, &mut |slot| {
                    self.slot_queue.push_back(slot);
                    num_enqueued_slots += 1;
                    num_pushes += 1;

                    // Split the work packet if needed.
                    if num_pushes >= Self::PUSHES_BEFORE_SPLIT
                        || self.slot_queue.len() >= Self::CAPACITY
                    {
                        self.split(worker);
                        num_splits += 1;
                        num_pushes = 0;
                    }
                });

                trace.post_scan_object(enqueued_object);
            }
        }

        probe!(
            mmtk,
            process_slots,
            num_initial_slots,
            num_visited_slots,
            num_enqueued_slots,
            num_empty_slots,
            num_scanned_objects,
            num_splits
        );
    }
}

/// A work packet for scanning objects and optionally do node-enqueuing tracing during a
/// stop-the-world tracing GC and the final mark pause of a concurrent GC.
///
/// It will scan each object.  For objects that supports slot enqueuing, it will collect their slots
/// and spawn [`ProcessSlots`] work packets to trace them.  For objects that don't support slot
/// enqueuing, it will immediately trace their slots and spawn other [`ProcessNodes`] work packets
/// to process their newly traced children.  It is the VM's responsibility to implement
/// [`Scanning::scan_object_and_trace_edges`] to update the references to point to the new addresses
/// in such a case.
pub struct ProcessNodes<T: Trace> {
    objects: Vec<ObjectReference>,
    bucket: WorkBucketStage,
    phantom_data: PhantomData<T>,
}

impl<T: Trace> ProcessNodes<T> {
    pub fn new(objects: Vec<ObjectReference>, bucket: WorkBucketStage) -> Self {
        Self {
            objects,
            bucket,
            phantom_data: PhantomData,
        }
    }

    fn try_enqueue_slots(
        &mut self,
        worker: &mut GCWorker<T::VM>,
        tls: VMWorkerThread,
        trace: &T,
    ) -> Vec<ObjectReference> {
        // We record objects that don't support slot-enqueuing tracing and process them later.
        let mut scan_later = Vec::new();

        let mut slots = VectorQueue::new();

        let flush = |slots: &mut VectorQueue<_>, worker: &mut GCWorker<T::VM>| {
            let buffer = slots.take();
            let work_packet = ProcessSlots::<T>::new(buffer, self.bucket);
            worker.add_work(self.bucket, work_packet);
        };

        // For any object we need to scan, we count its live bytes.
        // Check the option outside the loop for better performance.
        //
        // TODO: Currently all objects reached in a GC will be processed here,
        // so it is a good place to do statistics for all reachable objects.
        // In the future, when we refactor the ProcessNodes and ProcessSlots work packets
        // so that each of them can compute the transitive closure alone (i.e. removing double enqueuing),
        // we need to make sure both work packets will count the live bytes.
        if crate::util::rust_util::unlikely(*worker.mmtk.get_options().count_live_bytes_in_gc) {
            // Borrow before the loop.
            let mut live_bytes_stats = worker.shared.live_bytes_per_space.borrow_mut();
            for object in self.objects.iter().copied() {
                GCWorkerShared::<T::VM>::increase_live_bytes(&mut live_bytes_stats, object);
            }
        }

        for object in self.objects.iter().copied() {
            if <T::VM as VMBinding>::VMScanning::support_slot_enqueuing(tls, object) {
                trace!("Scan object (slot) {}", object);
                // If an object supports slot-enqueuing, we enqueue its slots.
                <T::VM as VMBinding>::VMScanning::scan_object(tls, object, &mut |slot| {
                    slots.push(slot);
                    if slots.is_full() {
                        flush(&mut slots, worker);
                    }
                });
                trace.post_scan_object(object);
            } else {
                // If an object does not support slot-enqueuing, we have to use
                // `Scanning::scan_object_and_trace_edges` and offload the job of updating the
                // reference field to the VM.
                //
                // TODO: We may refactor this work packet to do slot-enqueuing and node-enqueuing in
                // one loop.
                scan_later.push(object);
            }
        }

        if !slots.is_empty() {
            flush(&mut slots, worker);
        }

        scan_later
    }

    fn do_node_enqueuing_tracing(
        &mut self,
        worker: &mut GCWorker<T::VM>,
        tls: VMWorkerThread,
        trace: T,
        scan_later: Vec<ObjectReference>,
    ) {
        if scan_later.is_empty() {
            return;
        }

        let object_tracer_context = DefaultObjectTracerContext::<T>::new(self.bucket);

        object_tracer_context.with_tracer(worker, |object_tracer| {
            // Scan objects and trace their outgoing edges at the same time.
            for object in scan_later.iter().copied() {
                trace!("Scan object (node) {}", object);
                <T::VM as VMBinding>::VMScanning::scan_object_and_trace_edges(
                    tls,
                    object,
                    object_tracer,
                );
                trace.post_scan_object(object);
            }
        });
    }
}

impl<T: Trace> GCWork<T::VM> for ProcessNodes<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("ScanObjects");

        let tls = worker.tls;
        let trace = T::from_mmtk(mmtk);

        // Go through the object list and scan objects that supports slot-enququing.
        let scan_later = self.try_enqueue_slots(worker, tls, &trace);

        let total_objects = self.objects.len();
        let scan_and_trace = scan_later.len();
        probe!(mmtk, process_nodes, total_objects, scan_and_trace);

        // If any objects do not support slot-enqueuing, we process them now.
        self.do_node_enqueuing_tracing(worker, tls, trace, scan_later);

        trace!("ScanObjects End");
    }
}
