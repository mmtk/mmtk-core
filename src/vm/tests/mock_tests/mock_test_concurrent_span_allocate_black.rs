// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Allocate-black (design §5.8, Go's `gcmarknewobject`): objects allocated while
//! `allocate_as_live` is set are marked, counted in `bytes_marked`, and survive the cycle,
//! including objects in spans that are taken during marking.

use super::concurrent_span_util::*;
use super::mock_test_prelude::*;
use crate::policy::space::Space;

#[test]
pub fn allocate_black() {
    with_mockvm(
        default_setup,
        || {
            let fixture = MutatorFixture::create_with_heapsize(64 << 20);
            let Some(space) = span_space(fixture.mmtk()) else {
                eprintln!("No SpanSpace in this plan: test skipped");
                return;
            };
            let mutator = fixture.mutator();
            let sizes = [16usize, 64, 200, 3000, 16384];

            // Objects allocated before marking, never traced: they die.
            let mut before = vec![];
            for &size in sizes.iter() {
                for _ in 0..50 {
                    before.push((alloc_object(mutator, size), size));
                }
            }

            // "Concurrent marking": allocate black. Allocate enough to use the rest of the cached
            // spans and new spans.
            space.prepare_initial_mark();
            space.set_allocate_as_live(true);
            let mut during = vec![];
            for &size in sizes.iter() {
                for _ in 0..100 {
                    let marked_before = space.bytes_marked();
                    let obj = alloc_object(mutator, size);
                    assert!(obj.is_live(), "new object not allocated black");
                    assert_eq!(space.bytes_marked() - marked_before, elem_size(space, obj));
                    fill(obj, size, 0x77);
                    during.push((obj, size));
                }
            }
            for (o, _) in before.iter() {
                assert!(!o.is_live());
            }
            space.set_allocate_as_live(false);
            space.release_after_mark();
            release_span_allocator(mutator);
            sweep_all(space);

            for (o, size) in during.iter() {
                assert!(o.is_live());
                check_filled(*o, *size, 0x77);
            }
            for (o, _) in before.iter() {
                assert!(!o.is_live());
            }

            // After marking, new objects are not black: a cycle that marks nothing frees them
            // all.
            let after: Vec<_> = sizes
                .iter()
                .map(|&s| (alloc_object(mutator, s), s))
                .collect();
            cycle(space, mutator, &[]);
            for (o, _) in during.iter().chain(after.iter()) {
                assert!(!o.is_live());
            }
            assert_eq!(space.pages_in_use(), 0);
        },
        no_cleanup,
    )
}
