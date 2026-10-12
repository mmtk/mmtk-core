// GITHUB-CI: MMTK_PLAN=MarkSweep

//! MockVM can report root slots, so a GC keeps the objects they point to alive.

use super::mock_test_prelude::*;
use crate::util::{Address, ObjectReference};
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A root slot.  It holds an object reference as a raw address, or 0 for null.
static ROOT: AtomicUsize = AtomicUsize::new(0);

fn alloc(fixture: &MutatorFixture) -> ObjectReference {
    let size = 48;
    let addr = memory_manager::alloc(fixture.mutator(), size, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
    let objref = MockVM::object_start_to_ref(addr);
    memory_manager::post_alloc(
        fixture.mutator(),
        objref,
        size,
        AllocationSemantics::Default,
    );
    objref
}

#[test]
pub fn mocked_roots_keep_objects_alive() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                // Objects have no reference fields.
                scan_object: MockMethod::new_fixed(Box::new(|_| {})),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(8 * MB);
            let rooted = alloc(&fixture);
            let garbage = alloc(&fixture);
            ROOT.store(rooted.to_raw_address().as_usize(), Ordering::SeqCst);

            memory_manager::handle_user_collection_request(
                fixture.mmtk(),
                fixture.mutator_tls(),
                true,
            );

            assert!(memory_manager::is_live_object(rooted));
            assert!(!memory_manager::is_live_object(garbage));
        },
        no_cleanup,
    )
}
