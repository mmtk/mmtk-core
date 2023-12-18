// GITHUB-CI: MMTK_PLAN=GenImmix
// GITHUB-CI: FEATURES=vo_bit,extreme_assertions

use super::mock_test_prelude::*;

use crate::util::{Address, ObjectReference};
use atomic::Atomic;

lazy_static! {
    static ref FIXTURE: Fixture<SingleObject> = Fixture::new();
}

#[test]
#[should_panic(expected = "object bit is unset")]
fn test_assertion_barrier_invalid_ref() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture_mut(|fixture| {
                let objref = fixture.objref;

                // Create an edge
                let slot = Atomic::new(objref);
                let edge = Address::from_ref(&slot);

                // Create an invalid object reference (offset 8 bytes on the original object ref), and invoke barrier slowpath with it
                // The invalid object ref has no VO bit, and the assertion should fail.
                let invalid_objref =
                    ObjectReference::from_raw_address(objref.to_raw_address() + 8usize);
                fixture.mutator_mut().barrier.object_reference_write_slow(
                    invalid_objref,
                    edge,
                    objref,
                );
            });
        },
        no_cleanup,
    );
}

#[test]
fn test_assertion_barrier_valid_ref() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture_mut(|fixture| {
                let objref = fixture.objref;

                // Create an edge
                let slot = Atomic::new(objref);
                let edge = Address::from_ref(&slot);

                // Invoke barrier slowpath with the valid object ref
                fixture
                    .mutator_mut()
                    .barrier
                    .object_reference_write_slow(objref, edge, objref);
            });
        },
        no_cleanup,
    )
}
