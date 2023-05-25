// GITHUB-CI: MMTK_PLAN=GenImmix
// GITHUB-CI: FEATURES=vo_bit, extreme_assertions
use crate::object_model::OBJECT_REF_OFFSET;
use crate::{api::*, edges};
use atomic::Atomic;
use mmtk::util::{Address, ObjectReference, VMMutatorThread, VMThread};
use mmtk::vm::edge_shape::SimpleEdge;
use mmtk::AllocationSemantics;


#[test]
#[should_panic]
fn test_assertion_barrier() {
    const MB: usize = 1024 * 1024;
    // 1MB heap
    mmtk_init(1 * MB);
    let handle = mmtk_bind_mutator(VMMutatorThread(VMThread::UNINITIALIZED));
    mmtk_initialize_collection(VMThread::UNINITIALIZED);
    let size = 16;
    let addr = mmtk_alloc(handle, size, 8, 0, AllocationSemantics::Default);
    let objref: ObjectReference = ObjectReference::from_raw_address(addr.add(OBJECT_REF_OFFSET));
    let mut slot: Atomic<ObjectReference> = Atomic::new(objref);
    let edge = SimpleEdge::from_address(Address::from_ref(&mut slot));
    unsafe {
        let mu = &mut *handle;
        mu.barrier
            .object_reference_write_slow(objref, edges::DummyVMEdge::Simple(edge), objref);
    }
    
}
