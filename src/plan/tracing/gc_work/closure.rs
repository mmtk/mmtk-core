use std::{collections::VecDeque, marker::PhantomData};

use crate::{
    plan::{
        tracing::{SlotOfTrace, Trace},
        VectorQueue,
    },
    scheduler::{GCWork, GCWorker, GCWorkerShared, WorkBucketStage, EDGES_WORK_BUFFER_SIZE},
    util::{scanning_helper::visit_children_moving, ObjectReference, VMWorkerThread},
    vm::{slot::Slot, Scanning, VMBinding},
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

    pub fn from_vec(slots: Vec<SlotOfTrace<T>>, bucket: WorkBucketStage) -> Self {
        let mut slot_queue = VecDeque::from(slots);
        slot_queue.reserve(Self::CAPACITY.saturating_sub(slot_queue.len()));
        Self::from_deque(slot_queue, bucket)
    }

    pub fn from_deque(slot_queue: VecDeque<SlotOfTrace<T>>, bucket: WorkBucketStage) -> Self {
        Self { slot_queue, bucket }
    }

    fn try_enqueue_nodes(&mut self, worker: &mut GCWorker<T::VM>, _tls: VMWorkerThread, trace: &T) {
        let mut enqueued_objects = VecDeque::with_capacity(self.slot_queue.len());
        for slot in self.slot_queue.drain(..) {
            if let Some(target) = slot.load() {
                let new_target = trace.trace_object(worker, target, &mut |enqueued_object| {
                    enqueued_objects.push_back(enqueued_object);
                });
                if new_target != target {
                    slot.store(new_target);
                }
            }
        }

        if !enqueued_objects.is_empty() {
            let work_packet = ProcessNodes::<T>::from_deque(enqueued_objects, self.bucket);
            worker.add_work(self.bucket, work_packet);
        }
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

        if <T::VM as VMBinding>::VMScanning::prefer_node_enqueuing() {
            self.try_enqueue_nodes(worker, tls, &trace);
        }

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
    object_queue: VecDeque<ObjectReference>,
    bucket: WorkBucketStage,
    phantom_data: PhantomData<T>,
}

impl<T: Trace> ProcessNodes<T> {
    /// The nominative capacity of the deque.
    ///
    /// It will try to split the work packet if the deque has too many elements.
    const CAPACITY: usize = EDGES_WORK_BUFFER_SIZE / 4;

    /// The number of slots to push before it proactively splits the work packet.
    ///
    /// The purpose of this is preventing a pathological case where one worker keeps popping and
    /// pushing elements without reaching the capacity.
    const PUSHES_BEFORE_SPLIT: usize = Self::CAPACITY;

    pub fn from_vec(slots: Vec<ObjectReference>, bucket: WorkBucketStage) -> Self {
        let mut slot_queue = VecDeque::from(slots);
        slot_queue.reserve(Self::CAPACITY.saturating_sub(slot_queue.len()));
        Self::from_deque(slot_queue, bucket)
    }

    pub fn from_deque(object_queue: VecDeque<ObjectReference>, bucket: WorkBucketStage) -> Self {
        Self {
            object_queue,
            bucket,
            phantom_data: PhantomData,
        }
    }

    fn try_enqueue_slots(&mut self, worker: &mut GCWorker<T::VM>, tls: VMWorkerThread, trace: &T) {
        let mut slots = VectorQueue::new();

        let flush = |slots: &mut VectorQueue<_>, worker: &mut GCWorker<T::VM>| {
            let buffer = slots.take();
            let work_packet = ProcessSlots::<T>::from_vec(buffer, self.bucket);
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
            for object in self.object_queue.iter().copied() {
                GCWorkerShared::<T::VM>::increase_live_bytes(&mut live_bytes_stats, object);
            }
        }

        self.object_queue.retain(|object| {
            if <T::VM as VMBinding>::VMScanning::support_slot_enqueuing(tls, *object) {
                trace!("Scan object (slot) {}", object);
                // If an object supports slot-enqueuing, we enqueue its slots.
                <T::VM as VMBinding>::VMScanning::scan_object(tls, *object, &mut |slot| {
                    slots.push(slot);
                    if slots.is_full() {
                        flush(&mut slots, worker);
                    }
                });
                trace.post_scan_object(*object);
                false
            } else {
                true
            }
        });

        if !slots.is_empty() {
            flush(&mut slots, worker);
        }
    }

    fn do_node_enqueuing_tracing(
        &mut self,
        worker: &mut GCWorker<T::VM>,
        tls: VMWorkerThread,
        trace: T,
    ) {
        if self.object_queue.is_empty() {
            return;
        }

        let mut num_visited_nodes = 0;
        let mut num_visited_edges = 0;
        let mut num_pushes = 0;
        let mut num_splits = 0;

        while let Some(object) = self.object_queue.pop_back() {
            num_visited_nodes += 1;

            // Scan objects and trace their outgoing edges at the same time.
            visit_children_moving::<T::VM>(tls, object, &mut |target| {
                num_visited_edges += 1;
                let new_target = trace.trace_object(worker, target, &mut |enqueued_object| {
                    self.object_queue.push_back(enqueued_object);

                    num_pushes += 1;
                });
                new_target
            });

            trace.post_scan_object(object);

            // Split the work packet if needed.
            if num_pushes >= Self::PUSHES_BEFORE_SPLIT || self.object_queue.len() >= Self::CAPACITY
            {
                self.split(worker);
                num_splits += 1;
                num_pushes = 0;
            }
        }

        probe!(
            mmtk,
            do_node_enqueuing_tracing,
            num_visited_nodes,
            num_visited_edges,
            num_splits,
        );
    }

    fn split(&mut self, worker: &mut GCWorker<T::VM>) {
        debug_assert!(!self.object_queue.is_empty());

        let parked_workers_sloppy = worker.scheduler().worker_monitor.parked_workers_sloppy();

        // Divide the current work packet into two parts by default, but add one more part for every
        // idle worker.
        let divider = 2 + parked_workers_sloppy;

        // Each part has at least one element.
        let split_len = (self.object_queue.len() / divider).max(1);

        let mut remaining_parts = divider - 1; // Leave one part for myself.
        while remaining_parts > 0 && self.object_queue.len() > split_len {
            let split_queue = self.object_queue.split_off(split_len);
            let work = Self::from_deque(split_queue, self.bucket);
            worker.add_work(self.bucket, work);

            remaining_parts -= 1;
        }
    }
}

impl<T: Trace> GCWork<T::VM> for ProcessNodes<T> {
    fn do_work(&mut self, worker: &mut GCWorker<T::VM>, mmtk: &'static MMTK<T::VM>) {
        trace!("ScanObjects");

        let tls = worker.tls;
        let trace = T::from_mmtk(mmtk);

        let total_objects = self.object_queue.len();

        if !<T::VM as VMBinding>::VMScanning::prefer_node_enqueuing() {
            // Go through the object list and scan objects that supports slot-enququing.
            self.try_enqueue_slots(worker, tls, &trace)
        };

        let scan_and_trace = self.object_queue.len();
        probe!(mmtk, process_nodes, total_objects, scan_and_trace);

        // If any objects do not support slot-enqueuing, we process them now.
        self.do_node_enqueuing_tracing(worker, tls, trace);

        trace!("ScanObjects End");
    }
}
