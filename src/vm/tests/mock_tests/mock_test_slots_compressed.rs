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

/// This represents a location that holds a 32-bit pointer on a 64-bit machine.
///
/// OpenJDK uses this kind of slot to store compressed OOPs on 64-bit machines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CompressedOopSlot {
    slot_addr: *mut Atomic<u32>,
}

unsafe impl Send for CompressedOopSlot {}

impl CompressedOopSlot {
    pub fn from_address(address: Address) -> Self {
        Self {
            slot_addr: address.to_mut_ptr(),
        }
    }
    pub fn as_address(&self) -> Address {
        Address::from_mut_ptr(self.slot_addr)
    }
}

impl Slot for CompressedOopSlot {
    fn load(&self) -> Option<ObjectReference> {
        let compressed = unsafe { (*self.slot_addr).load(atomic::Ordering::Relaxed) };
        let expanded = (compressed as usize) << 3;
        ObjectReference::from_raw_address(unsafe { Address::from_usize(expanded) })
    }

    fn store(&self, object: ObjectReference) {
        let expanded = object.to_raw_address().as_usize();
        let compressed = (expanded >> 3) as u32;
        unsafe { (*self.slot_addr).store(compressed, atomic::Ordering::Relaxed) }
    }
}

// Two 35-bit addresses aligned to 8 bytes (3 zeros in the lowest bits).
const COMPRESSABLE_ADDR1: usize = 0b101_10111011_11011111_01111110_11111000usize;
const COMPRESSABLE_ADDR2: usize = 0b110_11110111_01101010_11011101_11101000usize;

#[test]
pub fn load_compressed() {
    // Note: We cannot guarantee GC will allocate an object in the low address region.
    // So we make up addresses just for testing the bit operations of compressed OOP slots.
    let compressed1 = (COMPRESSABLE_ADDR1 >> 3) as u32;
    let objref1 =
        ObjectReference::from_raw_address(unsafe { Address::from_usize(COMPRESSABLE_ADDR1) });

    let mut rust_slot: Atomic<u32> = Atomic::new(compressed1);

    let slot = CompressedOopSlot::from_address(Address::from_ref(&rust_slot));
    let objref = slot.load();

    assert_eq!(objref, objref1);
}

#[test]
pub fn store_compressed() {
    // Note: We cannot guarantee GC will allocate an object in the low address region.
    // So we make up addresses just for testing the bit operations of compressed OOP slots.
    let compressed1 = (COMPRESSABLE_ADDR1 >> 3) as u32;
    let compressed2 = (COMPRESSABLE_ADDR2 >> 3) as u32;
    let objref2 =
        ObjectReference::from_raw_address(unsafe { Address::from_usize(COMPRESSABLE_ADDR2) })
            .unwrap();

    let mut rust_slot: Atomic<u32> = Atomic::new(compressed1);

    let slot = CompressedOopSlot::from_address(Address::from_ref(&rust_slot));
    slot.store(objref2);
    assert_eq!(rust_slot.load(Ordering::SeqCst), compressed2);

    let objref = slot.load();
    assert_eq!(objref, Some(objref2));
}
