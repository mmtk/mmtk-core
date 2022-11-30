// GITHUB-CI: MMTK_PLAN=NoGC

use atomic::{Atomic, Ordering};
use mmtk::{
    util::{Address, ObjectReference},
    vm::edge_shape::{Edge, SimpleEdge},
};

use crate::{
    edges::{DummyVMEdge, OffsetEdge, TaggedEdge},
    tests::fixtures::{Fixture, TwoObjects},
};

#[cfg(target_pointer_width = "64")]
use crate::edges::only_64_bit::CompressedOopEdge;

lazy_static! {
    static ref FIXTURE: Fixture<TwoObjects> = Fixture::new();
}

#[test]
pub fn load_simple() {
    FIXTURE.with_fixture(|fixture| {
        let mut slot: Atomic<ObjectReference> = Atomic::new(fixture.objref1);

        let edge = SimpleEdge::from_address(Address::from_ref(&mut slot));
        let objref = edge.load();

        assert_eq!(objref, fixture.objref1);
    });
}

#[test]
pub fn store_simple() {
    FIXTURE.with_fixture(|fixture| {
        let mut slot: Atomic<ObjectReference> = Atomic::new(fixture.objref1);

        let edge = SimpleEdge::from_address(Address::from_ref(&mut slot));
        edge.store(fixture.objref2);
        assert_eq!(slot.load(Ordering::SeqCst), fixture.objref2);

        let objref = edge.load();
        assert_eq!(objref, fixture.objref2);
    });
}

#[cfg(target_pointer_width = "64")]
mod only_64_bit {
    use super::*;

    // Two 35-bit addresses aligned to 8 bytes (3 zeros in the lowest bits).
    const COMPRESSABLE_ADDR1: usize = 0b101_10111011_11011111_01111110_11111000usize;
    const COMPRESSABLE_ADDR2: usize = 0b110_11110111_01101010_11011101_11101000usize;

    #[test]
    pub fn load_compressed() {
        // Note: We cannot guarantee GC will allocate an object in the low address region.
        // So we make up addresses just for testing the bit operations of compressed OOP edges.
        let compressed1 = (COMPRESSABLE_ADDR1 >> 3) as u32;
        let objref1 = ObjectReference::from_raw_address(unsafe { Address::from_usize(COMPRESSABLE_ADDR1) });

        let mut slot: Atomic<u32> = Atomic::new(compressed1);

        let edge = CompressedOopEdge::from_address(Address::from_ref(&mut slot));
        let objref = edge.load();

        assert_eq!(objref, objref1);
    }

    #[test]
    pub fn store_compressed() {
        // Note: We cannot guarantee GC will allocate an object in the low address region.
        // So we make up addresses just for testing the bit operations of compressed OOP edges.
        let compressed1 = (COMPRESSABLE_ADDR1 >> 3) as u32;
        let compressed2 = (COMPRESSABLE_ADDR2 >> 3) as u32;
        let objref2 = ObjectReference::from_raw_address(unsafe { Address::from_usize(COMPRESSABLE_ADDR2) });

        let mut slot: Atomic<u32> = Atomic::new(compressed1);

        let edge = CompressedOopEdge::from_address(Address::from_ref(&mut slot));
        edge.store(objref2);
        assert_eq!(slot.load(Ordering::SeqCst), compressed2);

        let objref = edge.load();
        assert_eq!(objref, objref2);
    }
}

#[test]
pub fn load_offset() {
    const OFFSET: usize = 48;
    FIXTURE.with_fixture(|fixture| {
        let addr1 = fixture.objref1.to_raw_address();
        let mut slot: Atomic<Address> = Atomic::new(addr1 + OFFSET);

        let edge = OffsetEdge::new_with_offset(Address::from_ref(&mut slot), OFFSET);
        let objref = edge.load();

        assert_eq!(objref, fixture.objref1);
    });
}

#[test]
pub fn store_offset() {
    const OFFSET: usize = 48;
    FIXTURE.with_fixture(|fixture| {
        let addr1 = fixture.objref1.to_raw_address();
        let addr2 = fixture.objref2.to_raw_address();
        let mut slot: Atomic<Address> = Atomic::new(addr1 + OFFSET);

        let edge = OffsetEdge::new_with_offset(Address::from_ref(&mut slot), OFFSET);
        edge.store(fixture.objref2);
        assert_eq!(slot.load(Ordering::SeqCst), addr2 + OFFSET);

        let objref = edge.load();
        assert_eq!(objref, fixture.objref2);
    });
}

const TAG1: usize = 0b01;
const TAG2: usize = 0b10;

#[test]
pub fn load_tagged() {
    FIXTURE.with_fixture(|fixture| {
        let mut slot1: Atomic<usize> = Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG1);
        let mut slot2: Atomic<usize> = Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG2);

        let edge1 = TaggedEdge::new(Address::from_ref(&mut slot1));
        let edge2 = TaggedEdge::new(Address::from_ref(&mut slot2));
        let objref1 = edge1.load();
        let objref2 = edge2.load();

        // Tags should not affect loaded values.
        assert_eq!(objref1, fixture.objref1);
        assert_eq!(objref2, fixture.objref1);
    });
}

#[test]
pub fn store_tagged() {
    FIXTURE.with_fixture(|fixture| {
        let mut slot1: Atomic<usize> = Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG1);
        let mut slot2: Atomic<usize> = Atomic::new(fixture.objref1.to_raw_address().as_usize() | TAG2);

        let edge1 = TaggedEdge::new(Address::from_ref(&mut slot1));
        let edge2 = TaggedEdge::new(Address::from_ref(&mut slot2));
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
}

#[test]
pub fn mixed() {
    const OFFSET: usize = 48;

    FIXTURE.with_fixture(|fixture| {
        let addr1 = fixture.objref1.to_raw_address();
        let addr2 = fixture.objref2.to_raw_address();

        let mut slot1: Atomic<ObjectReference> = Atomic::new(fixture.objref1);
        let mut slot3: Atomic<Address> = Atomic::new(addr1 + OFFSET);
        let mut slot4: Atomic<usize> = Atomic::new(addr1.as_usize() | TAG1);

        let edge1 = SimpleEdge::from_address(Address::from_ref(&mut slot1));
        let edge3 = OffsetEdge::new_with_offset(Address::from_ref(&mut slot3), OFFSET);
        let edge4 = TaggedEdge::new(Address::from_ref(&mut slot4));

        let de1 = DummyVMEdge::Simple(edge1);
        let de3 = DummyVMEdge::Offset(edge3);
        let de4 = DummyVMEdge::Tagged(edge4);

        let edges = vec![de1, de3, de4];
        for (i, edge) in edges.iter().enumerate() {
            let objref = edge.load();
            assert_eq!(objref, fixture.objref1, "Edge {} is not properly loaded", i);
        }

        let mutable_edges = vec![de1, de3, de4];
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
}
