// GITHUB-CI: MMTK_PLAN=NoGC

#![allow(unused)]

use super::mock_test_prelude::*;
use crate::{
    util::{Address, ObjectReference},
    vm::edge_shape::{Edge, SimpleEdge},
};
use atomic::{Atomic, Ordering};

lazy_static! {
    static ref FIXTURE: Fixture<TwoObjects> = Fixture::new();
}

mod simple_edges {
    use super::*;

    #[test]
    pub fn load_simple() {
        with_mockvm(
            default_setup,
            || {
                FIXTURE.with_fixture(|fixture| {
                    let mut slot: Atomic<ObjectReference> = Atomic::new(fixture.objref1);

                    let edge = SimpleEdge::from_address(Address::from_ref(&slot));
                    let objref = edge.load();

                    assert_eq!(objref, fixture.objref1);
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
                    let mut slot: Atomic<ObjectReference> = Atomic::new(fixture.objref1);

                    let edge = SimpleEdge::from_address(Address::from_ref(&slot));
                    edge.store(fixture.objref2);
                    assert_eq!(slot.load(Ordering::SeqCst), fixture.objref2);

                    let objref = edge.load();
                    assert_eq!(objref, fixture.objref2);
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
    /// OpenJDK uses this kind of edge to store compressed OOPs on 64-bit machines.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct CompressedOopEdge {
        slot_addr: *mut Atomic<u32>,
    }

    unsafe impl Send for CompressedOopEdge {}

    impl CompressedOopEdge {
        pub fn from_address(address: Address) -> Self {
            Self {
                slot_addr: address.to_mut_ptr(),
            }
        }
        pub fn as_address(&self) -> Address {
            Address::from_mut_ptr(self.slot_addr)
        }
    }

    impl Edge for CompressedOopEdge {
        fn load(&self) -> ObjectReference {
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
        // So we make up addresses just for testing the bit operations of compressed OOP edges.
        let compressed1 = (COMPRESSABLE_ADDR1 >> 3) as u32;
        let objref1 =
            ObjectReference::from_raw_address(unsafe { Address::from_usize(COMPRESSABLE_ADDR1) });

        let mut slot: Atomic<u32> = Atomic::new(compressed1);

        let edge = CompressedOopEdge::from_address(Address::from_ref(&slot));
        let objref = edge.load();

        assert_eq!(objref, objref1);
    }

    #[test]
    pub fn store_compressed() {
        // Note: We cannot guarantee GC will allocate an object in the low address region.
        // So we make up addresses just for testing the bit operations of compressed OOP edges.
        let compressed1 = (COMPRESSABLE_ADDR1 >> 3) as u32;
        let compressed2 = (COMPRESSABLE_ADDR2 >> 3) as u32;
        let objref2 =
            ObjectReference::from_raw_address(unsafe { Address::from_usize(COMPRESSABLE_ADDR2) });

        let mut slot: Atomic<u32> = Atomic::new(compressed1);

        let edge = CompressedOopEdge::from_address(Address::from_ref(&slot));
        edge.store(objref2);
        assert_eq!(slot.load(Ordering::SeqCst), compressed2);

        let objref = edge.load();
        assert_eq!(objref, objref2);
    }
}

mod offset_edge {
    use super::*;

    /// This represents an edge that holds a pointer to the *middle* of an object, and the offset is known.
    ///
    /// Julia uses this trick to facilitate deleting array elements from the front.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct OffsetEdge {
        slot_addr: *mut Atomic<Address>,
        offset: usize,
    }

    unsafe impl Send for OffsetEdge {}

    impl OffsetEdge {
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

    impl Edge for OffsetEdge {
        fn load(&self) -> ObjectReference {
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
                    let mut slot: Atomic<Address> = Atomic::new(addr1 + OFFSET);

                    let edge = OffsetEdge::new_with_offset(Address::from_ref(&slot), OFFSET);
                    let objref = edge.load();

                    assert_eq!(objref, fixture.objref1);
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
                    let mut slot: Atomic<Address> = Atomic::new(addr1 + OFFSET);

                    let edge = OffsetEdge::new_with_offset(Address::from_ref(&slot), OFFSET);
                    edge.store(fixture.objref2);
                    assert_eq!(slot.load(Ordering::SeqCst), addr2 + OFFSET);

                    let objref = edge.load();
                    assert_eq!(objref, fixture.objref2);
                });
            },
            no_cleanup,
        )
    }
}

mod tagged_edge {
    use super::*;

    /// This edge presents the object reference itself to mmtk-core.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct TaggedEdge {
        slot_addr: *mut Atomic<usize>,
    }

    unsafe impl Send for TaggedEdge {}

    impl TaggedEdge {
        // The DummyVM has OBJECT_REF_OFFSET = 4.
        // Using a two-bit tag should be safe on both 32-bit and 64-bit platforms.
        const TAG_BITS_MASK: usize = 0b11;

        pub fn new(address: Address) -> Self {
            Self {
                slot_addr: address.to_mut_ptr(),
            }
        }
    }

    impl Edge for TaggedEdge {
        fn load(&self) -> ObjectReference {
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
                    let mut slot1: Atomic<usize> =
                        Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG1);
                    let mut slot2: Atomic<usize> =
                        Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG2);

                    let edge1 = TaggedEdge::new(Address::from_ref(&slot1));
                    let edge2 = TaggedEdge::new(Address::from_ref(&slot2));
                    let objref1 = edge1.load();
                    let objref2 = edge2.load();

                    // Tags should not affect loaded values.
                    assert_eq!(objref1, fixture.objref1);
                    assert_eq!(objref2, fixture.objref1);
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
                    let mut slot1: Atomic<usize> =
                        Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG1);
                    let mut slot2: Atomic<usize> =
                        Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG2);

                    let edge1 = TaggedEdge::new(Address::from_ref(&slot1));
                    let edge2 = TaggedEdge::new(Address::from_ref(&slot2));
                    edge1.store(fixture.objref2);
                    edge2.store(fixture.objref2);

                    // Tags should be preserved.
                    assert_eq!(
                        slot1.load(Ordering::SeqCst),
                        fixture.objref2.to_raw_address().as_usize() | TAG1
                    );
                    assert_eq!(
                        slot2.load(Ordering::SeqCst),
                        fixture.objref2.to_raw_address().as_usize() | TAG2
                    );

                    let objref1 = edge1.load();
                    let objref2 = edge2.load();

                    // Tags should not affect loaded values.
                    assert_eq!(objref1, fixture.objref2);
                    assert_eq!(objref2, fixture.objref2);
                });
            },
            no_cleanup,
        )
    }
}

mod mixed {
    #[cfg(target_pointer_width = "64")]
    use super::compressed_oop::CompressedOopEdge;
    use super::offset_edge::OffsetEdge;
    use super::offset_edge::OFFSET;
    use super::tagged_edge::TaggedEdge;
    use super::tagged_edge::TAG1;
    use super::*;
    use crate::vm::edge_shape::SimpleEdge;

    /// If a VM supports multiple kinds of edges, we can use tagged union to represent all of them.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum DummyVMEdge {
        Simple(SimpleEdge),
        #[cfg(target_pointer_width = "64")]
        Compressed(compressed_oop::CompressedOopEdge),
        Offset(OffsetEdge),
        Tagged(TaggedEdge),
    }

    unsafe impl Send for DummyVMEdge {}

    impl Edge for DummyVMEdge {
        fn load(&self) -> ObjectReference {
            match self {
                DummyVMEdge::Simple(e) => e.load(),
                #[cfg(target_pointer_width = "64")]
                DummyVMEdge::Compressed(e) => e.load(),
                DummyVMEdge::Offset(e) => e.load(),
                DummyVMEdge::Tagged(e) => e.load(),
            }
        }

        fn store(&self, object: ObjectReference) {
            match self {
                DummyVMEdge::Simple(e) => e.store(object),
                #[cfg(target_pointer_width = "64")]
                DummyVMEdge::Compressed(e) => e.store(object),
                DummyVMEdge::Offset(e) => e.store(object),
                DummyVMEdge::Tagged(e) => e.store(object),
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

                    let mut slot1: Atomic<ObjectReference> = Atomic::new(fixture.objref1);
                    let mut slot3: Atomic<Address> = Atomic::new(addr1 + OFFSET);
                    let mut slot4: Atomic<usize> = Atomic::new(addr1.as_usize() | TAG1);

                    let edge1 = SimpleEdge::from_address(Address::from_ref(&slot1));
                    let edge3 = OffsetEdge::new_with_offset(Address::from_ref(&slot3), OFFSET);
                    let edge4 = TaggedEdge::new(Address::from_ref(&slot4));

                    let de1 = DummyVMEdge::Simple(edge1);
                    let de3 = DummyVMEdge::Offset(edge3);
                    let de4 = DummyVMEdge::Tagged(edge4);

                    let edges = [de1, de3, de4];
                    for (i, edge) in edges.iter().enumerate() {
                        let objref = edge.load();
                        assert_eq!(objref, fixture.objref1, "Edge {} is not properly loaded", i);
                    }

                    let mutable_edges = [de1, de3, de4];
                    for (i, edge) in mutable_edges.iter().enumerate() {
                        edge.store(fixture.objref2);
                        let objref = edge.load();
                        assert_eq!(
                            objref, fixture.objref2,
                            "Edge {} is not properly loaded after store",
                            i
                        );
                    }

                    assert_eq!(slot1.load(Ordering::SeqCst), fixture.objref2);
                    assert_eq!(slot3.load(Ordering::SeqCst), addr2 + OFFSET);
                });
            },
            no_cleanup,
        )
    }
}
