// GITHUB-CI: MMTK_PLAN=NoGC,MarkSweep,MarkCompact,SemiSpace,Immix
// GITHUB-CI: FEATURES=vo_bit

// Note on the plans chosen for CI:
// - Those plans cover the MarkSweepSpace, MarkCompactSpace, CopySpace and ImmixSpace.
//   Each plan other than NoGC also include ImmortalSpace and LOS.
// - PageProtect consumes too much memory and the test will fail with the default heap size
//   chosen by the MutatorFixture.

use std::collections::HashSet;

use constants::BYTES_IN_WORD;

use super::mock_test_prelude::*;

use crate::{util::*, AllocationSemantics, MMTK};

lazy_static! {
    static ref FIXTURE: Fixture<MutatorFixture> = Fixture::new();
}

pub fn get_all_objects(mmtk: &'static MMTK<MockVM>) -> HashSet<ObjectReference> {
    let mut result = HashSet::new();
    mmtk.enumerate_objects(|object: EnumeratedObject| {
        result.insert(match object {
            EnumeratedObject::Single { reference, .. } => reference,
            EnumeratedObject::InBlock { reference, .. } => reference,
        });
    });
    result
}

#[test]
pub fn test_heap_traversal() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture_mut(|fixture| {
                let mmtk = fixture.mmtk();
                let traversal0 = get_all_objects(mmtk);
                assert!(traversal0.is_empty());

                let mutator = &mut fixture.mutator;

                let align = BYTES_IN_WORD;

                let mut new_obj = |size: usize, semantics: AllocationSemantics| {
                    let start = memory_manager::alloc(mutator, size, align, 0, semantics);
                    let object = MockVM::object_start_to_ref(start);
                    memory_manager::post_alloc(mutator, object, size, semantics);
                    object
                };

                let mut known_objects = HashSet::new();

                let mut new_and_assert = |size: usize, semantics: AllocationSemantics| {
                    let object = new_obj(size, semantics); // a random size
                    known_objects.insert(object);
                    let traversal = get_all_objects(mmtk);
                    assert_eq!(traversal, known_objects);
                };

                {
                    use AllocationSemantics::*;

                    // Add some single objects.  Size doesn't matter.
                    new_and_assert(40, Default);
                    new_and_assert(64, Default);
                    new_and_assert(96, Immortal);
                    new_and_assert(131000, Los);

                    // Allocate enough memory to fill up a 64KB Immix block
                    for _ in 0..1000 {
                        new_and_assert(112, Default);
                    }
                    // Allocate a few objects in the immortal space.
                    for _ in 0..10 {
                        new_and_assert(64, Immortal);
                    }
                    // And a few objects in the LOS.
                    for _ in 0..3 {
                        // The MutatorFixture only has 1MB of memory.  Don't allocate too much.
                        new_and_assert(65504, Immortal);
                    }
                }
            });
        },
        no_cleanup,
    )
}
