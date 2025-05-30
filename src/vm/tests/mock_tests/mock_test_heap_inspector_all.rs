// GITHUB-CI: MMTK_PLAN=all
// GITHUB-CI: FEATURES=vo_bit

use std::collections::HashSet;

use constants::BYTES_IN_WORD;

use super::mock_test_prelude::*;

use crate::{util::*, AllocationSemantics, MMTK};

lazy_static! {
    static ref FIXTURE: Fixture<MutatorFixture> = Fixture::new();
}

pub fn get_all_objects(mmtk: &'static MMTK<MockVM>) -> HashSet<ObjectReference> {
    let mut result = HashSet::new();
    let space_inspectors = mmtk.inspect_spaces();
    assert!(space_inspectors.len() > 0);
    space_inspectors.iter().for_each(|s| {
        let mut regions = s.list_top_regions();
        while regions.len() > 0 {
            let region = regions.pop().unwrap();
            let mut sub_regions = s.list_sub_regions(&*region);
            if sub_regions.len() > 0 {
                // If we have sub regions keep looking at them
                regions.append(&mut sub_regions);
            } else {
                // Otherwise, we are at the leaf level, listing objects.
                for object in region.list_objects() {
                    assert!(!result.contains(&object));
                    result.insert(object);
                }
            }
        }
    });
    return result;
}

#[test]
pub fn test_heap_inspector_all() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture_mut(|fixture| {
                let mmtk = fixture.mmtk();
                let mutator = &mut fixture.mutator;

                let mut new_obj = |size: usize, semantics: AllocationSemantics| {
                    let align = BYTES_IN_WORD;
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
                    println!("Allocating object 40");
                    new_and_assert(40, Default);
                    println!("Allocating object 64");
                    new_and_assert(64, Default);
                    println!("Allocating object 96");
                    new_and_assert(96, Immortal);
                    println!("Allocating object 131000");
                    new_and_assert(131000, Los);
                }
            });
        },
        no_cleanup,
    )
}
