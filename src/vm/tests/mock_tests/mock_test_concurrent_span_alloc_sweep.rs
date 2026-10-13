// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! SpanSpace allocation, sweeping and mark epochs, driven by hand (no roots can be mocked yet):
//! each "cycle" is `prepare_initial_mark`, tracing a chosen set of objects,
//! `release_after_mark`, and releasing the mutator's span allocator, as the plan does in a pause.

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;
use crate::util::{Address, ObjectReference};

const SIZES: [usize; 12] = [
    16, 24, 48, 100, 512, 1000, 1500, 4000, 9000, 20000, 28000, 32768,
];

#[test]
pub fn alloc_sweep_epochs() {
    with_mockvm(
        default_setup,
        || {
            let fixture = MutatorFixture::create_with_heapsize(64 << 20);
            let Some(space) = span_space(fixture.mmtk()) else {
                eprintln!("No SpanSpace in this plan: test skipped");
                return;
            };
            let mutator = fixture.mutator();

            // Allocate several spans' worth of objects of each size, and fill them with a pattern.
            let mut objects: Vec<(ObjectReference, usize)> = vec![];
            for &size in SIZES.iter() {
                for _ in 0..(3 * (65536 / size)).clamp(4, 300) {
                    let obj = alloc_object(mutator, size);
                    fill(obj, size, 0xA5);
                    objects.push((obj, size));
                }
            }
            assert_disjoint(&objects);

            // Keep every other object.
            let live: Vec<_> = objects.iter().step_by(2).cloned().collect();
            let dead: Vec<_> = objects.iter().skip(1).step_by(2).cloned().collect();

            // Cycle 1.
            assert_eq!(space.prepare_initial_mark(), 0, "nothing to sweep yet");
            let expected_marked: usize = live.iter().map(|(o, _)| elem_size(space, *o)).sum();
            mark(space, &live);
            assert_eq!(space.bytes_marked(), expected_marked);
            // Tracing again marks nothing new.
            mark(space, &live);
            assert_eq!(space.bytes_marked(), expected_marked);
            for (o, _) in live.iter() {
                assert!(o.is_live());
            }
            for (o, _) in dead.iter() {
                assert!(!o.is_live());
            }
            space.release_after_mark();
            assert_eq!(space.heap_live(), expected_marked);
            release_span_allocator(mutator);
            // Full spans returned to the central sets before the cycle ended are now unswept.
            assert!(space.has_unswept());
            // Liveness is the same before and after sweeping.
            for (o, _) in live.iter() {
                assert!(o.is_live());
            }
            for (o, _) in dead.iter() {
                assert!(!o.is_live());
            }

            // Allocate again without sweeping first: the allocator sweeps lazily, and new objects
            // never overlap live ones. New objects are zeroed.
            let unswept_before = space.count_unswept();
            let mut new_objects = vec![];
            for &size in SIZES.iter() {
                for _ in 0..(65536 / size).clamp(2, 100) {
                    let obj = alloc_object(mutator, size);
                    assert_zeroed(obj, size);
                    fill(obj, size, 0x5A);
                    new_objects.push((obj, size));
                }
            }
            assert!(
                space.count_unswept() < unswept_before,
                "allocation swept some spans lazily"
            );
            let mut all: Vec<_> = live.clone();
            all.extend(new_objects.iter().cloned());
            assert_disjoint(&all);
            for (o, size) in live.iter() {
                check_filled(*o, *size, 0xA5);
            }

            // Finish sweeping without allocating.
            let mut swept = 0;
            loop {
                let pages = space.sweep_one();
                if pages == 0 {
                    break;
                }
                swept += pages;
            }
            assert!(!space.has_unswept());
            assert!(swept > 0 || unswept_before == 0);
            for (o, _) in live.iter().chain(new_objects.iter()) {
                assert!(o.is_live());
            }
            for (o, size) in live.iter() {
                check_filled(*o, *size, 0xA5);
            }

            // Cycle 2: the marks of cycle 1 must not survive into cycle 2. Mark only the new
            // objects; everything from before dies.
            assert_eq!(space.prepare_initial_mark(), 0);
            assert_eq!(space.bytes_marked(), 0);
            for (o, _) in live.iter() {
                assert!(!o.is_live(), "stale mark from the previous cycle");
            }
            mark(space, &new_objects);
            space.release_after_mark();
            release_span_allocator(mutator);
            sweep_all(space);
            for (o, _) in live.iter() {
                assert!(!o.is_live());
            }
            for (o, size) in new_objects.iter() {
                assert!(o.is_live());
                check_filled(*o, *size, 0x5A);
            }

            // Cycle 3: nothing survives, and all memory goes back to MMTk.
            space.prepare_initial_mark();
            space.release_after_mark();
            release_span_allocator(mutator);
            sweep_all(space);
            assert_eq!(space.pages_in_use(), 0);
            assert_eq!(reserved_data_pages(space), 0);
        },
        no_cleanup,
    )
}

fn assert_disjoint(objects: &[(ObjectReference, usize)]) {
    let mut ranges: Vec<(Address, Address)> = objects
        .iter()
        .map(|(o, size)| {
            let start = object_start(*o);
            (start, start + *size)
        })
        .collect();
    ranges.sort();
    for w in ranges.windows(2) {
        assert!(
            w[0].1 <= w[1].0,
            "overlapping objects {:?} {:?}",
            w[0],
            w[1]
        );
    }
}
