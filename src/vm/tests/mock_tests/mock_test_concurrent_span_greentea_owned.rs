// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Green Tea's span ownership protocol (Go: `tryAcquire`, `release`, `scanSpan`), driven directly
//! on `SpanSpace` without the scheduler:
//!
//! - one thread: the first marker of a span queues it, later markers do not, and the span scan
//!   scans all of them; a mark after the scan released the span queues it again;
//! - several threads mark every object of the same spans in different orders, hand queued spans
//!   to each other, and scan them while others keep marking.  Every object is scanned exactly
//!   once, objects allocated black are never scanned, and no span stays owned.
//!
//! Skipped when Green Tea is off (`MMTK_CONCURRENT_SPAN_GREEN_TEA=false`).

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;
use crate::policy::sft::SFT;
use crate::policy::space::Space;
use crate::policy::spanspace::{SpanQueueTrace, SpanSpace, SpanWork};
use crate::util::ObjectReference;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const SIZE: usize = 48;
const NUM_OBJECTS: usize = 4000;
const NUM_BLACK: usize = 500;
const NUM_THREADS: usize = 4;

fn queue_of(
    space: &SpanSpace<MockVM>,
    object: ObjectReference,
    queued: &mut Vec<SpanWork>,
) -> bool {
    match space.trace_object_for_span_queue(object) {
        SpanQueueTrace::ScanSpan(work) => {
            queued.push(work);
            true
        }
        SpanQueueTrace::Done => false,
        SpanQueueTrace::ScanObject => panic!("{object} should use the span queue"),
    }
}

#[test]
pub fn green_tea_ownership() {
    with_mockvm(
        default_setup,
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(32 * MB);
            let Some(space) = span_space(fixture.mmtk()) else {
                return;
            };
            if !space.green_tea() {
                info!("Green Tea is off: skipped");
                return;
            }
            let mutator = fixture.mutator();
            let objects: Vec<ObjectReference> = (0..NUM_OBJECTS)
                .map(|_| alloc_object(mutator, SIZE))
                .collect();
            let index: HashMap<ObjectReference, usize> =
                objects.iter().enumerate().map(|(i, o)| (*o, i)).collect();
            let span_count = {
                let mut starts: Vec<_> = objects
                    .iter()
                    .map(|o| space.span_of(object_start(*o)).unwrap().start())
                    .collect();
                starts.sort();
                starts.dedup();
                starts.len()
            };

            // Start marking, and allocate some objects black.
            space.prepare_initial_mark();
            space.set_allocate_as_live(true);
            let black: Vec<ObjectReference> = (0..NUM_BLACK)
                .map(|_| alloc_object(mutator, SIZE))
                .collect();
            space.set_allocate_as_live(false);

            // One thread.  The span of objects[0] holds objects[0..3].
            let a = objects[0];
            let span_a = space.span_of(object_start(a)).unwrap();
            assert!(std::ptr::eq(
                span_a,
                space.span_of(object_start(objects[2])).unwrap()
            ));
            let mut queued = vec![];
            assert!(queue_of(space, a, &mut queued));
            assert!(span_a.is_owned());
            // Marked again: nothing to do.
            assert!(!queue_of(space, a, &mut queued));
            // Marked while the span is owned: left to the owner.
            assert!(!queue_of(space, objects[1], &mut queued));
            let mut scanned = vec![];
            assert_eq!(
                space.scan_span(queued.pop().unwrap(), |o| scanned.push(o)),
                2
            );
            scanned.sort();
            let mut expected = vec![a, objects[1]];
            expected.sort();
            assert_eq!(scanned, expected);
            assert!(!span_a.is_owned());
            // A mark after the release queues the span again.  Only that object is scanned.
            assert!(queue_of(space, objects[2], &mut queued));
            let mut scanned = vec![];
            space.scan_span(queued.pop().unwrap(), |o| scanned.push(o));
            assert_eq!(scanned, vec![objects[2]]);

            // Several threads.
            let scans: Vec<AtomicUsize> = (0..NUM_OBJECTS).map(|_| AtomicUsize::new(0)).collect();
            let shared: Mutex<Vec<SpanWork>> = Mutex::new(vec![]);
            let acquired = AtomicUsize::new(0);
            let batched = AtomicUsize::new(0);
            let stolen = AtomicUsize::new(0);
            for o in &objects[0..3] {
                scans[index[o]].fetch_add(1, Ordering::Relaxed);
            }
            std::thread::scope(|s| {
                for t in 0..NUM_THREADS {
                    let objects = &objects;
                    let (scans, shared, index) = (&scans, &shared, &index);
                    let (acquired, batched, stolen) = (&acquired, &batched, &stolen);
                    s.spawn(move || {
                        // Each thread marks every object, in its own order.
                        let mut order: Vec<usize> = (0..NUM_OBJECTS).collect();
                        let mut seed = 0x1234_5678_9ABC_DEF0u64 ^ (t as u64 * 0x9E37_79B9);
                        for i in (1..order.len()).rev() {
                            seed ^= seed << 13;
                            seed ^= seed >> 7;
                            seed ^= seed << 17;
                            order.swap(i, (seed % (i as u64 + 1)) as usize);
                        }
                        let mut local: Vec<SpanWork> = vec![];
                        let scan = |work: SpanWork, own: bool| {
                            let n = space.scan_span(work, |o| {
                                scans[index[&o]].fetch_add(1, Ordering::Relaxed);
                            });
                            if n > 1 {
                                batched.fetch_add(1, Ordering::Relaxed);
                            }
                            if !own {
                                stolen.fetch_add(1, Ordering::Relaxed);
                            }
                        };
                        for (k, i) in order.into_iter().enumerate() {
                            if queue_of(space, objects[i], &mut local) {
                                acquired.fetch_add(1, Ordering::Relaxed);
                            }
                            // Hand every other span to the shared queue, and scan now and then,
                            // so spans are scanned while other threads still mark them.
                            if local.len() >= 2 {
                                shared.lock().unwrap().push(local.remove(0));
                            }
                            if k % 7 == 0 {
                                if let Some(w) = local.pop() {
                                    scan(w, true);
                                }
                                let w = shared.lock().unwrap().pop();
                                if let Some(w) = w {
                                    scan(w, false);
                                }
                            }
                        }
                        while let Some(w) = local.pop() {
                            scan(w, true);
                        }
                        loop {
                            let w = shared.lock().unwrap().pop();
                            match w {
                                Some(w) => scan(w, false),
                                None => break,
                            }
                        }
                    });
                }
            });
            assert!(shared.lock().unwrap().is_empty());

            let acquired = acquired.load(Ordering::Relaxed);
            info!(
                "{span_count} spans, {acquired} acquisitions, {} batched scans, {} scans of shared spans",
                batched.load(Ordering::Relaxed),
                stolen.load(Ordering::Relaxed)
            );
            for (i, o) in objects.iter().enumerate() {
                assert_eq!(
                    scans[i].load(Ordering::Relaxed),
                    1,
                    "object {i} ({o}) scanned a wrong number of times"
                );
                assert!(space.is_live(*o));
            }
            for (_, span) in objects
                .iter()
                .chain(black.iter())
                .map(|o| (o, space.span_of(object_start(*o)).unwrap()))
            {
                assert!(!span.is_owned());
                assert_eq!(span.count_marked_unscanned(), 0);
            }
            // Objects allocated black are marked and scanned (never by a span scan).
            for o in &black {
                let span = space.span_of(object_start(*o)).unwrap();
                let i = span.obj_index(object_start(*o));
                assert!(span.is_marked(i) && span.is_scanned(i));
            }
            // Spans were queued again after being scanned.
            assert!(acquired >= span_count);

            // Finish the cycle: everything survives.
            space.release_after_mark();
            release_span_allocator(mutator);
            sweep_all(space);
            for o in objects.iter().chain(black.iter()) {
                assert!(space.is_live(*o));
            }
        },
        no_cleanup,
    )
}
