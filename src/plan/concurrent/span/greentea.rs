//! Green Tea span scanning for `ConcurrentSpan` (design §3.4, §5.9, §5.12; Go:
//! `runtime/mgcmark_greenteagc.go` and `gcDrain` in `runtime/mgcmark.go` (2c3c1caad2)).
//!
//! [`SpanTraceObjects`] is the concurrent marking packet of `ConcurrentSpan` while the option
//! `concurrent_span_green_tea` is on.  Every generic concurrent marking packet
//! (`ConcurrentTraceObjects` from the roots, barrier flushes, rescans, finalizable contents, and
//! the leftovers that `FinalMark` finishes in the pause) hands its objects to it through
//! [`crate::plan::ConcurrentPlan::trace_objects_concurrently`].
//!
//! A packet has two local queues, as Go's `gcWork` has:
//! - an object queue (LIFO), for objects that are scanned one by one: objects of larger size
//!   classes, and objects outside `SpanSpace`;
//! - a span queue (FIFO ring of 256), for spans of small objects
//!   ([`crate::policy::spanspace::SpanClass::uses_span_queue`]).  Marking such an object queues
//!   its span only if the marker takes ownership of the span; the span is then scanned once for
//!   every object marked in it in the meantime ([`SpanSpace::scan_span`]).
//!
//! Work is taken in Go's order: local objects first, then local spans, so that spans accumulate
//! marked objects before they are scanned.  Go's shared queues (workbufs on `work.full`, and the
//! stealable span rings of other Ps) are MMTk work packets: spilled spans and object queue overflow
//! become new [`SpanTraceObjects`] packets, which other workers take from the bucket or steal
//! from this worker's local packet buffer (deviation GT-3).

use crate::plan::concurrent::global::ConcurrentPlan;
use crate::plan::concurrent::span::global::ConcurrentSpan;
use crate::plan::concurrent::Pause;
use crate::plan::tracing::{PlanTrace, Trace};
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::policy::space::Space;
use crate::policy::spanspace::{SpanQueueTrace, SpanSpace, SpanWork};
use crate::scheduler::{GCWork, GCWorker, WorkBucketStage};
use crate::util::{scanning_helper, ObjectReference};
use crate::vm::{ObjectModel, VMBinding};
use crate::MMTK;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Go: `len(spanQueue.ring)`.
const SPAN_RING_SIZE: usize = 256;
/// Go: `spanQueue.put`'s `spillPeriod`: how often (in puts) to check whether to spill spans.
const SPILL_PERIOD: usize = 64;
/// Go: `spillMax`: at most this many spans are spilled by a periodic spill.
const SPILL_MAX: usize = 16;
/// The object queue overflow, as in `ConcurrentTraceObjects`.
const OBJECT_PACKET_SIZE: usize = 8192;
const OBJECT_QUEUE_OVERFLOW: usize = OBJECT_PACKET_SIZE * 2;

/// Trace `objects` with Green Tea.  Called through
/// [`ConcurrentPlan::trace_objects_concurrently`].
pub(super) fn trace_objects<VM: VMBinding>(
    worker: &mut GCWorker<VM>,
    objects: Vec<ObjectReference>,
    already_marked: bool,
) {
    let mmtk = worker.mmtk;
    let mut packet = SpanTraceObjects::<VM>::new(objects, already_marked, vec![]);
    packet.do_work(worker, mmtk);
}

/// A concurrent marking packet with a span queue and an object queue (design §5.9, §5.12).
pub(super) struct SpanTraceObjects<VM: VMBinding> {
    /// Objects to trace (or, if `already_marked`, to scan).
    objects: Vec<ObjectReference>,
    already_marked: bool,
    /// Spans to scan, spilled by another packet.  Their ownership was taken by the marker that
    /// queued them.
    spans: Vec<SpanWork>,
    /// Set when this packet starts, so that the packet that spilled it knows its spans have been
    /// taken (Go: `spanQueue.chainEmpty`).
    started: Option<Arc<AtomicBool>>,
    _p: std::marker::PhantomData<VM>,
}

unsafe impl<VM: VMBinding> Send for SpanTraceObjects<VM> {}

impl<VM: VMBinding> SpanTraceObjects<VM> {
    pub(super) fn new(
        objects: Vec<ObjectReference>,
        already_marked: bool,
        spans: Vec<SpanWork>,
    ) -> Self {
        Self {
            objects,
            already_marked,
            spans,
            started: None,
            _p: std::marker::PhantomData,
        }
    }
}

impl<VM: VMBinding> GCWork<VM> for SpanTraceObjects<VM> {
    fn do_work(&mut self, worker: &mut GCWorker<VM>, mmtk: &'static MMTK<VM>) {
        if let Some(started) = self.started.take() {
            started.store(true, Ordering::Relaxed);
        }
        let plan = mmtk
            .get_plan()
            .downcast_ref::<ConcurrentSpan<VM>>()
            .unwrap();
        let mut queues = Queues::<VM> {
            space: &plan.span,
            trace: PlanTrace::<ConcurrentSpan<VM>, DEFAULT_TRACE>::from_mmtk(mmtk),
            objects: vec![],
            spans: VecDeque::with_capacity(SPAN_RING_SIZE),
            puts_since_spill: 0,
            last_spill: None,
            // Overflow stays in the pause when `FinalMark` finishes marking, where `Concurrent` is
            // closed (as in `ConcurrentTraceObjects`).
            stage: if plan.current_pause() == Some(Pause::FinalMark) {
                WorkBucketStage::FinishConcurrentWork
            } else {
                WorkBucketStage::Concurrent
            },
        };

        let objects = std::mem::take(&mut self.objects);
        if self.already_marked {
            queues.objects.extend(objects);
        } else {
            for object in objects {
                queues.trace(worker, object);
            }
        }
        // Spilled spans fit in the ring (at most half of it).
        debug_assert!(self.spans.len() <= SPAN_RING_SIZE);
        queues.spans.extend(std::mem::take(&mut self.spans));

        let scan_work = queues.drain(worker);
        if scan_work > 0 {
            plan.add_scan_work(scan_work, 0, 0);
        }
    }
}

/// The local queues of a [`SpanTraceObjects`] packet (Go: `gcWork`'s workbufs and `spanQueue`).
struct Queues<VM: VMBinding> {
    space: &'static SpanSpace<VM>,
    trace: PlanTrace<ConcurrentSpan<VM>, DEFAULT_TRACE>,
    /// Marked objects to scan.
    objects: Vec<ObjectReference>,
    /// Queued spans to scan (Go: `spanQueue.ring`).
    spans: VecDeque<SpanWork>,
    /// Go: `spanQueue.putsSinceDrain`.
    puts_since_spill: usize,
    /// The packet of the last spill, until it starts (Go: the `spanSPMC` chain).
    last_spill: Option<Arc<AtomicBool>>,
    /// Where spilled work goes.
    stage: WorkBucketStage,
}

impl<VM: VMBinding> Queues<VM> {
    /// Go: `gcDrain`'s loop, local part.  Returns the scan work done (bytes).
    fn drain(&mut self, worker: &mut GCWorker<VM>) -> usize {
        let mut scan_work = 0;
        loop {
            // Go: `tryGetObjFast`, then `tryGetSpanFast`.
            if let Some(object) = self.objects.pop() {
                scan_work += self.scan_object(worker, object);
                if self.objects.len() >= OBJECT_QUEUE_OVERFLOW {
                    let overflow = self.objects.drain(..OBJECT_PACKET_SIZE).collect();
                    worker.add_work(
                        self.stage,
                        SpanTraceObjects::<VM>::new(overflow, true, vec![]),
                    );
                }
            } else if let Some(work) = self.spans.pop_front() {
                let space = self.space;
                let _n = space.scan_span(work, |object| {
                    scan_work += self.scan_object(worker, object);
                });
                #[cfg(test)]
                {
                    stats::SPANS_SCANNED.fetch_add(1, Ordering::Relaxed);
                    stats::SPAN_OBJECTS_SCANNED.fetch_add(_n, Ordering::Relaxed);
                    if _n > 1 {
                        stats::SPAN_SCANS_BATCHED.fetch_add(1, Ordering::Relaxed);
                    }
                }
            } else {
                break;
            }
        }
        scan_work
    }

    /// Scan one object (Go: `scanobject` or `scanObjectSmall`), tracing its children.  Returns
    /// the scan work: the object size, as `ConcurrentTraceObjects` counts it (deviation PLAN-1).
    #[inline(always)]
    fn scan_object(&mut self, worker: &mut GCWorker<VM>, object: ObjectReference) -> usize {
        let tls = worker.tls;
        scanning_helper::visit_children_non_moving::<VM>(tls, object, &mut |child| {
            self.trace(worker, child);
            child
        });
        self.trace.post_scan_object(object);
        VM::VMObjectModel::get_current_size(object)
    }

    /// Go: `tryDeferToSpanScan`, else `findObject` + `greyobject`.
    #[inline(always)]
    fn trace(&mut self, worker: &mut GCWorker<VM>, object: ObjectReference) {
        if self.space.in_space(object) {
            match self.space.trace_object_for_span_queue(object) {
                SpanQueueTrace::Done => {}
                SpanQueueTrace::ScanObject => self.objects.push(object),
                SpanQueueTrace::ScanSpan(work) => self.put_span(worker, work),
            }
        } else {
            let objects = &mut self.objects;
            self.trace
                .trace_object(worker, object, &mut |o: ObjectReference| objects.push(o));
        }
    }

    /// Go: `spanQueue.put`.
    fn put_span(&mut self, worker: &mut GCWorker<VM>, work: SpanWork) {
        if self.spans.len() < SPAN_RING_SIZE {
            self.spans.push_back(work);
            // Occasionally try to spill some work to generate parallelism.
            self.puts_since_spill += 1;
            if self.puts_since_spill >= SPILL_PERIOD {
                // Reset even if we don't spill, so we don't check every time.
                self.puts_since_spill = 0;
                // Don't bother if there are very few spans, or if spilled spans are still
                // waiting to be taken.
                let n = usize::min(self.spans.len() / 2, SPILL_MAX);
                let chain_empty = self
                    .last_spill
                    .as_ref()
                    .is_none_or(|started| started.load(Ordering::Relaxed));
                if n > 4 && chain_empty {
                    self.spill(worker, n);
                }
            }
            return;
        }
        // We're out of space.  Spill half of the local spans.
        self.spill(worker, SPAN_RING_SIZE / 2);
        self.spans.push_back(work);
    }

    /// Go: `spanQueue.drain`: move the `n` oldest spans to a new packet that any worker can take.
    fn spill(&mut self, worker: &mut GCWorker<VM>, n: usize) {
        self.puts_since_spill = 0;
        let spans: Vec<SpanWork> = self.spans.drain(..n).collect();
        #[cfg(test)]
        stats::SPANS_SPILLED.fetch_add(n, Ordering::Relaxed);
        let mut packet = SpanTraceObjects::<VM>::new(vec![], true, spans);
        let started = Arc::new(AtomicBool::new(false));
        packet.started = Some(started.clone());
        self.last_spill = Some(started);
        worker.add_work(self.stage, packet);
    }
}

/// Counters for tests.
#[cfg(test)]
pub(crate) mod stats {
    use std::sync::atomic::AtomicUsize;
    /// Spans taken from a span queue and scanned.
    pub static SPANS_SCANNED: AtomicUsize = AtomicUsize::new(0);
    /// Span scans that scanned more than one object.
    pub static SPAN_SCANS_BATCHED: AtomicUsize = AtomicUsize::new(0);
    /// Objects scanned by span scans.
    pub static SPAN_OBJECTS_SCANNED: AtomicUsize = AtomicUsize::new(0);
    /// Spans spilled into new packets.
    pub static SPANS_SPILLED: AtomicUsize = AtomicUsize::new(0);
}
