// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! `HybridBarrier` logs both the old and the new value of a reference store, but only while its
//! global flag is set.

use super::mock_test_prelude::*;

use crate::plan::concurrent::barrier::HybridBarrierSemantics;
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::HybridBarrier;
use crate::policy::gc_work::DEFAULT_TRACE;
use crate::util::{Address, ObjectReference};
use crate::AllocationSemantics;
use atomic::Atomic;

type Barrier = HybridBarrier<HybridBarrierSemantics<MockVM, ConcurrentSpan<MockVM>, DEFAULT_TRACE>>;

fn alloc(fixture: &MutatorFixture) -> ObjectReference {
    let size = 48;
    let addr = memory_manager::alloc(fixture.mutator(), size, 8, 0, AllocationSemantics::Default);
    assert!(!addr.is_zero());
    let objref = MockVM::object_start_to_ref(addr);
    memory_manager::post_alloc(
        fixture.mutator(),
        objref,
        size,
        AllocationSemantics::Default,
    );
    objref
}

fn buffered(fixture: &MutatorFixture) -> Vec<ObjectReference> {
    let barrier = fixture.mutator().barrier.downcast_ref::<Barrier>().unwrap();
    barrier.semantics().buffered().to_vec()
}

#[test]
pub fn hybrid_barrier_logs_old_and_new_values() {
    with_mockvm(
        default_setup,
        || {
            let fixture = MutatorFixture::create();
            if fixture
                .mutator()
                .barrier
                .downcast_ref::<Barrier>()
                .is_none()
            {
                eprintln!("ConcurrentSpan does not use HybridBarrier (SATB option): test skipped");
                return;
            }
            let src = alloc(&fixture);
            let old = alloc(&fixture);
            let new = alloc(&fixture);

            let slot_storage = Atomic::new(old);
            let slot = Address::from_ref(&slot_storage);

            // The flag is not set: the barrier does nothing.
            assert!(!Barrier::is_active());
            memory_manager::object_reference_write_pre(fixture.mutator(), src, slot, Some(new));
            assert!(buffered(&fixture).is_empty());

            // Set the flag: the old and the new value are logged, in that order.
            Barrier::set_active(true);
            assert_eq!(
                unsafe { memory_manager::barrier_active_address().load::<u8>() },
                1
            );
            memory_manager::object_reference_write_pre(fixture.mutator(), src, slot, Some(new));
            assert_eq!(buffered(&fixture), vec![old, new]);

            // Storing null logs only the old value.
            memory_manager::object_reference_write_pre(fixture.mutator(), src, slot, None);
            assert_eq!(buffered(&fixture), vec![old, new, old]);

            // Flushing outside concurrent marking drops the buffer.
            Barrier::set_active(false);
            memory_manager::flush_mutator(fixture.mutator());
            assert!(buffered(&fixture).is_empty());
        },
        no_cleanup,
    )
}
