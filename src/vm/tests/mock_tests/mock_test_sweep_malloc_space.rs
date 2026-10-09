// GITHUB-CI: MMTK_PLAN=MarkSweep
// GITHUB-CI: FEATURES=malloc_mark_sweep

// MallocSpace allocates objects with libc malloc, so the object addresses are not mapped
// by MMTk's mmapper. Sweeping linearly scans each chunk by VO bits, and must still find
// every object. With no roots, all the objects are dead and must be freed by the GC.

use super::mock_test_prelude::*;

use crate::util::constants::BYTES_IN_WORD;
use crate::util::metadata::vo_bit;
use crate::util::ObjectReference;
use crate::AllocationSemantics;

#[test]
pub fn malloc_ms_sweep_dead_objects() {
    with_mockvm(
        default_setup,
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(16 * MB);
            let mutator = fixture.mutator();

            let objects: Vec<ObjectReference> = (0..100)
                .map(|_| {
                    let size = 64;
                    let semantics = AllocationSemantics::Default;
                    let start = memory_manager::alloc(mutator, size, BYTES_IN_WORD, 0, semantics);
                    assert!(!start.is_zero());
                    let object = MockVM::object_start_to_ref(start);
                    memory_manager::post_alloc(mutator, object, size, semantics);
                    object
                })
                .collect();

            for object in objects.iter() {
                assert!(vo_bit::is_vo_bit_set(*object));
            }

            // No roots. All the objects are dead.
            memory_manager::handle_user_collection_request(
                fixture.mmtk(),
                fixture.mutator_tls(),
                false,
            );

            let not_swept: Vec<_> = objects
                .iter()
                .filter(|o| vo_bit::is_vo_bit_set(**o))
                .collect();
            assert!(
                not_swept.is_empty(),
                "{} of {} dead objects were not swept, e.g. {}",
                not_swept.len(),
                objects.len(),
                not_swept[0]
            );
        },
        no_cleanup,
    )
}
