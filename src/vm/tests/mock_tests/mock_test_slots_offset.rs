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

/// This represents a slot that holds a pointer to the *middle* of an object, and the offset is known.
///
/// Julia uses this trick to facilitate deleting array elements from the front.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OffsetSlot {
    slot_addr: *mut Atomic<Address>,
    offset: usize,
}

unsafe impl Send for OffsetSlot {}

impl OffsetSlot {
    pub fn new_no_offset(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
            offset: 0,
        }
    }

    pub fn new_with_offset(address: Address, offset: usize) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
            offset,
        }
    }

    pub fn slot_address(&self) -> Address {
        Address::from_mut_ptr(self.slot_addr)
    }

    pub fn offset(&self) -> usize {
        self.offset
    }
}

impl Slot for OffsetSlot {
    fn load(&self) -> Option<ObjectReference> {
        let middle = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        let begin = middle - self.offset;
        ObjectReference::from_raw_address(begin)
    }

    fn store(&self, object: ObjectReference) {
        let begin = object.to_raw_address();
        let middle = begin + self.offset;
        unsafe { (*self.slot_addr).store(middle, atomic::Ordering::Relaxed) }
    }
}

pub const OFFSET: usize = 48;

#[test]
pub fn load_offset() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture(|fixture| {
                let addr1 = fixture.objref1.to_raw_address();
                let mut rust_slot: Atomic<Address> = Atomic::new(addr1 + OFFSET);

                let slot = OffsetSlot::new_with_offset(Address::from_ref(&rust_slot), OFFSET);
                let objref = slot.load();

                assert_eq!(objref, Some(fixture.objref1));
            });
        },
        no_cleanup,
    )
}

#[test]
pub fn store_offset() {
    with_mockvm(
        default_setup,
        || {
            FIXTURE.with_fixture(|fixture| {
                let addr1 = fixture.objref1.to_raw_address();
                let addr2 = fixture.objref2.to_raw_address();
                let mut rust_slot: Atomic<Address> = Atomic::new(addr1 + OFFSET);

                let slot = OffsetSlot::new_with_offset(Address::from_ref(&rust_slot), OFFSET);
                slot.store(fixture.objref2);
                assert_eq!(rust_slot.load(Ordering::SeqCst), addr2 + OFFSET);

                let objref = slot.load();
                assert_eq!(objref, Some(fixture.objref2));
            });
        },
        no_cleanup,
    )
}
