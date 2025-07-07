// GITHUB-CI: MMTK_PLAN=Immix
// GITHUB-CI: FEATURES=vo_bit

use constants::BYTES_IN_WORD;

use super::mock_test_prelude::*;

use crate::{util::*, AllocationSemantics};

lazy_static! {
    static ref FIXTURE: Fixture<MutatorFixture> = Fixture::new();
}

#[test]
pub fn test_heap_inspector_immix() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture_mut(|fixture| {
                let mmtk = fixture.mmtk();
                let mutator = &mut fixture.mutator;
                let space_inspector = mmtk.inspect_spaces();
                assert!(space_inspector.len() > 0);

                let get_immix_inspector = || {
                    space_inspector.iter().find(|s| s.space_name() == "immix").unwrap()
                };

                {
                    let immix_space_inspector = get_immix_inspector();
                    assert_eq!(immix_space_inspector.policy_name(), "mmtk::policy::immix::immixspace::ImmixSpace<mmtk::util::test_util::mock_vm::MockVM>");
                    let chunk_inspector = immix_space_inspector.list_top_regions();
                    assert_eq!(chunk_inspector.len(), 0);
                }

                let mut new_obj = |size: usize, semantics: AllocationSemantics| {
                    let align = BYTES_IN_WORD;
                    let start = memory_manager::alloc(mutator, size, align, 0, semantics);
                    let object = MockVM::object_start_to_ref(start);
                    memory_manager::post_alloc(mutator, object, size, semantics);
                    object
                };

                // Allocate one object
                let object = new_obj(40, AllocationSemantics::Default);

                {
                    let immix_space_inspector = get_immix_inspector();
                    // Check chunks
                    let chunk_inspector = immix_space_inspector.list_top_regions();
                    assert_eq!(chunk_inspector.len(), 1);
                    assert_eq!(chunk_inspector[0].region_type(), "mmtk::util::heap::chunk_map::Chunk");
                    let objects = chunk_inspector[0].list_objects();
                    assert_eq!(objects.len(), 1);
                    assert_eq!(objects[0], object);
                    // Check blocks
                    let block_inspector = immix_space_inspector.list_sub_regions(&*chunk_inspector[0]);
                    assert_eq!(block_inspector.len(), 128); // 128 blocks in a chunk
                    assert_eq!(block_inspector[0].region_type(), "mmtk::policy::immix::block::Block");
                    let objects = block_inspector[0].list_objects();
                    assert_eq!(objects.len(), 1);
                    assert_eq!(objects[0], object);
                    // Check lines
                    let line_inspector = immix_space_inspector.list_sub_regions(&*block_inspector[0]);
                    assert_eq!(line_inspector.len(), 128); // 128 lines in a block
                    assert_eq!(line_inspector[0].region_type(), "mmtk::policy::immix::line::Line");
                    let objects = line_inspector[0].list_objects();
                    assert_eq!(objects.len(), 1);
                    assert_eq!(objects[0], object);
                }

                // Allocate another object
                let object2 = new_obj(40, AllocationSemantics::Default);

                {
                    let immix_space_inspector = get_immix_inspector();
                    // Check checks
                    let chunk_inspector = immix_space_inspector.list_top_regions();
                    assert_eq!(chunk_inspector.len(), 1);
                    assert_eq!(chunk_inspector[0].region_type(), "mmtk::util::heap::chunk_map::Chunk");
                    let objects = chunk_inspector[0].list_objects();
                    assert_eq!(objects.len(), 2);
                    assert_eq!(objects[0], object);
                    assert_eq!(objects[1], object2);
                }
            });
        },
        no_cleanup,
    )
}
