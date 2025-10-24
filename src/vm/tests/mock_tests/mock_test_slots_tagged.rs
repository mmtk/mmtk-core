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

/// This slot represents a slot that holds a tagged pointer.
/// The last two bits are tag bits and are not part of the object reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TaggedSlot {
    slot_addr: *mut Atomic<usize>,
}

unsafe impl Send for TaggedSlot {}

impl TaggedSlot {
    // The DummyVM has OBJECT_REF_OFFSET = 4.
    // Using a two-bit tag should be safe on both 32-bit and 64-bit platforms.
    const TAG_BITS_MASK: usize = 0b11;

    pub fn new(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
        }
    }
}

impl Slot for TaggedSlot {
    fn load(&self) -> Option<ObjectReference> {
        let tagged = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        let untagged = tagged & !Self::TAG_BITS_MASK;
        ObjectReference::from_raw_address(unsafe { Address::from_usize(untagged) })
    }

    fn store(&self, object: ObjectReference) {
        let old_tagged = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        let new_untagged = object.to_raw_address().as_usize();
        let new_tagged = new_untagged | (old_tagged & Self::TAG_BITS_MASK);
        unsafe { (*self.slot_addr).store(new_tagged, atomic::Ordering::Relaxed) }
    }
}

pub const TAG1: usize = 0b01;
pub const TAG2: usize = 0b10;

#[test]
pub fn load_tagged() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture(|fixture| {
                let mut rust_slot1: Atomic<usize> =
                    Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG1);
                let mut rust_slot2: Atomic<usize> =
                    Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG2);

                let slot1 = TaggedSlot::new(Address::from_ref(&rust_slot1));
                let slot2 = TaggedSlot::new(Address::from_ref(&rust_slot2));
                let objref1 = slot1.load();
                let objref2 = slot2.load();

                // Tags should not affect loaded values.
                assert_eq!(objref1, Some(fixture.objref1));
                assert_eq!(objref2, Some(fixture.objref1));
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn store_tagged() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture(|fixture| {
                let mut rust_slot1: Atomic<usize> =
                    Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG1);
                let mut rust_slot2: Atomic<usize> =
                    Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG2);

                let slot1 = TaggedSlot::new(Address::from_ref(&rust_slot1));
                let slot2 = TaggedSlot::new(Address::from_ref(&rust_slot2));
                slot1.store(fixture.objref2);
                slot2.store(fixture.objref2);

                // Tags should be preserved.
                assert_eq!(
                    rust_slot1.load(Ordering::SeqCst),
                    fixture.objref2.to_raw_address().as_usize() | TAG1
                );
                assert_eq!(
                    rust_slot2.load(Ordering::SeqCst),
                    fixture.objref2.to_raw_address().as_usize() | TAG2
                );

                let objref1 = slot1.load();
                let objref2 = slot2.load();

                // Tags should not affect loaded values.
                assert_eq!(objref1, Some(fixture.objref2));
                assert_eq!(objref2, Some(fixture.objref2));
            });
        },
        no_cleanup,
    )
}
