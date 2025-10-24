// GITHUB-CI: MMTK_PLAN=NoGC

#![allow(unused)]

use super::mock_test_prelude::*;
use crate::{
    util::{Address, ObjectReference},
    vm::slot::{SimpleSlot, Slot},
};
use atomic::{Atomic, Ordering};

lazy_static! {
    static ref FIXTURE: Fixture<TwoObjects> = Fixture::new();
}

#[test]
pub fn load_simple() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture(|fixture| {
                let mut rust_slot: Atomic<ObjectReference> = Atomic::new(fixture.objref1);

                let slot = SimpleSlot::from_address(Address::from_ref(&rust_slot));
                let objref = slot.load();

                assert_eq!(objref, Some(fixture.objref1));
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn store_simple() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture(|fixture| {
                let mut rust_slot: Atomic<ObjectReference> = Atomic::new(fixture.objref1);

                let slot = SimpleSlot::from_address(Address::from_ref(&rust_slot));
                slot.store(fixture.objref2);
                assert_eq!(rust_slot.load(Ordering::SeqCst), fixture.objref2);

                let objref = slot.load();
                assert_eq!(objref, Some(fixture.objref2));
            });
        },
        no_cleanup,
    )
}
