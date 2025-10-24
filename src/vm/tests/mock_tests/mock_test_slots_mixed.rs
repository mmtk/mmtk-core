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

#[cfg(target_pointer_width = "64")]
use super::mock_test_slots_compressed::CompressedOopSlot;
use super::mock_test_slots_offset::OffsetSlot;
use super::mock_test_slots_offset::OFFSET;
use super::mock_test_slots_tagged::TaggedSlot;
use super::mock_test_slots_tagged::TAG1;

/// If a VM supports multiple kinds of slots, we can use tagged union to represent all of them.
/// This is for testing, only.  A Rust `enum` may not be the most efficient representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DummyVMSlot {
    Simple(SimpleSlot),
    #[cfg(target_pointer_width = "64")]
    Compressed(CompressedOopSlot),
    Offset(OffsetSlot),
    Tagged(TaggedSlot),
}

unsafe impl Send for DummyVMSlot {}

impl Slot for DummyVMSlot {
    fn load(&self) -> Option<ObjectReference> {
        match self {
            DummyVMSlot::Simple(e) => e.load(),
            #[cfg(target_pointer_width = "64")]
            DummyVMSlot::Compressed(e) => e.load(),
            DummyVMSlot::Offset(e) => e.load(),
            DummyVMSlot::Tagged(e) => e.load(),
        }
    }

    fn store(&self, object: ObjectReference) {
        match self {
            DummyVMSlot::Simple(e) => e.store(object),
            #[cfg(target_pointer_width = "64")]
            DummyVMSlot::Compressed(e) => e.store(object),
            DummyVMSlot::Offset(e) => e.store(object),
            DummyVMSlot::Tagged(e) => e.store(object),
        }
    }
}

#[test]
pub fn mixed() {
    with_mockvm(
        default_setup,
        || {
            const OFFSET: usize = 48;

            FIXTURE.with_fixture(|fixture| {
                let addr1 = fixture.objref1.to_raw_address();
                let addr2 = fixture.objref2.to_raw_address();

                let mut rust_slot1: Atomic<ObjectReference> = Atomic::new(fixture.objref1);
                let mut rust_slot3: Atomic<Address> = Atomic::new(addr1 + OFFSET);
                let mut rust_slot4: Atomic<usize> = Atomic::new(addr1.as_usize() | TAG1);

                let slot1 = SimpleSlot::from_address(Address::from_ref(&rust_slot1));
                let slot3 = OffsetSlot::new_with_offset(Address::from_ref(&rust_slot3), OFFSET);
                let slot4 = TaggedSlot::new(Address::from_ref(&rust_slot4));

                let ds1 = DummyVMSlot::Simple(slot1);
                let ds3 = DummyVMSlot::Offset(slot3);
                let ds4 = DummyVMSlot::Tagged(slot4);

                let slots = [ds1, ds3, ds4];
                for (i, slot) in slots.iter().enumerate() {
                    let objref = slot.load();
                    assert_eq!(
                        objref,
                        Some(fixture.objref1),
                        "Slot {} is not properly loaded",
                        i
                    );
                }

                let mutable_slots = [ds1, ds3, ds4];
                for (i, slot) in mutable_slots.iter().enumerate() {
                    slot.store(fixture.objref2);
                    let objref = slot.load();
                    assert_eq!(
                        objref,
                        Some(fixture.objref2),
                        "Slot {} is not properly loaded after store",
                        i
                    );
                }

                assert_eq!(rust_slot1.load(Ordering::SeqCst), fixture.objref2);
                assert_eq!(rust_slot3.load(Ordering::SeqCst), addr2 + OFFSET);
            });
        },
        no_cleanup,
    )
}
