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

mod simple_slots {
    use super::*;

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
}

#[cfg(target_pointer_width = "64")]
mod compressed_oop {
    use super::*;

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
}

mod offset_slot {
    use super::*;

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
}

mod tagged_slot {
    use super::*;

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
}

mod mixed {
    #[cfg(target_pointer_width = "64")]
    use super::compressed_oop::CompressedOopSlot;
    use super::offset_slot::OffsetSlot;
    use super::offset_slot::OFFSET;
    use super::tagged_slot::TaggedSlot;
    use super::tagged_slot::TAG1;
    use super::*;
    use crate::vm::slot::SimpleSlot;

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
}
