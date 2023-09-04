// GITHUB-CI: MMTK_PLAN=GenImmix
// GITHUB-CI: FEATURES=vo_bit,extreme_assertions

// Run the test with any plan that uses object barrier, and we also need both VO bit and extreme assertions.

use crate::object_model::OBJECT_REF_OFFSET;
use crate::test_fixtures::FixtureContent;
use crate::test_fixtures::MMTKSingleton;
use crate::{api::*, edges};
use atomic::Atomic;
use mmtk::util::{Address, ObjectReference};
use mmtk::util::{VMMutatorThread, VMThread};
use mmtk::vm::edge_shape::SimpleEdge;
use mmtk::AllocationSemantics;

lazy_static! {
    static ref MMTK_SINGLETON: MMTKSingleton = MMTKSingleton::create();
}

#[test]
#[should_panic(expected = "object bit is unset")]
fn test_assertion_barrier_invalid_ref() {
    let mutator = mmtk_bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));

    // Allocate
    let size = 24;
    let addr = mmtk_alloc(mutator, size, 8, 0, AllocationSemantics::Default);
    let objref: ObjectReference = ObjectReference::from_raw_address(addr.add(OBJECT_REF_OFFSET));
    mmtk_post_alloc(mutator, objref, size, AllocationSemantics::Default);
    // Create an edge
    let mut slot: Atomic<ObjectReference> = Atomic::new(objref);
    let edge = SimpleEdge::from_address(Address::from_ref(&mut slot));
    // Create an invalid object reference (offset 8 bytes on the original object ref), and invoke barrier slowpath with it
    // The invalid object ref has no VO bit, and the assertion should fail.
    let invalid_objref = ObjectReference::from_raw_address(objref.to_raw_address() + 8usize);
    unsafe {
        let mu = &mut *mutator;
        mu.barrier.object_reference_write_slow(
            invalid_objref,
            edges::DummyVMEdge::Simple(edge),
            objref,
        );
    }
}

#[test]
fn test_assertion_barrier_valid_ref() {
    let mutator = mmtk_bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));

    // Allocate
    let size = 24;
    let addr = mmtk_alloc(mutator, size, 8, 0, AllocationSemantics::Default);
    let objref: ObjectReference = ObjectReference::from_raw_address(addr.add(OBJECT_REF_OFFSET));
    mmtk_post_alloc(mutator, objref, size, AllocationSemantics::Default);
    // Create an edge
    let mut slot: Atomic<ObjectReference> = Atomic::new(objref);
    let edge = SimpleEdge::from_address(Address::from_ref(&mut slot));
    // Invoke barrier slowpath with the valid object ref
    unsafe {
        let mu = &mut *mutator;
        mu.barrier
            .object_reference_write_slow(objref, edges::DummyVMEdge::Simple(edge), objref);
    }
}
