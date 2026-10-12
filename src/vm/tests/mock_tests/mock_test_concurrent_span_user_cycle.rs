// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! A user-requested collection runs a concurrent cycle that blocks only the caller, like Go's
//! `runtime.GC()`: it returns after the cycle has ended, without a stop-the-world `Full` pause.

use super::mock_test_prelude::*;
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::ConcurrentPlan;
use crate::util::{Address, ObjectReference};
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicUsize, Ordering};

static ROOT: AtomicUsize = AtomicUsize::new(0);

fn field(object: ObjectReference) -> Address {
    object.to_raw_address() + 8usize
}

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
    unsafe { field(objref).store(0usize) };
    objref
}

#[test]
pub fn user_gc_is_a_concurrent_cycle() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                get_object_size: MockMethod::new_fixed(Box::new(|_| 48)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    visitor.visit_slot(field(object));
                })),
                supports_blocking_mutators: MockMethod::new_fixed(Box::new(|_| true)),
                ..MockVM::default()
            }
        },
        || {
            let fixture = MutatorFixture::create_with_heapsize(8 << 20);
            let mmtk = fixture.mmtk();
            let plan = mmtk
                .get_plan()
                .downcast_ref::<ConcurrentSpan<MockVM>>()
                .unwrap();
            for round in 0..3 {
                let a = alloc(&fixture);
                let b = alloc(&fixture);
                let garbage = alloc(&fixture);
                unsafe { field(a).store(b) };
                ROOT.store(a.to_raw_address().as_usize(), Ordering::SeqCst);

                let cycles = plan.completed_cycles();
                assert!(memory_manager::handle_user_collection_request(
                    mmtk,
                    fixture.mutator_tls(),
                    true
                ));
                // The whole cycle is over when the request returns.
                assert_eq!(plan.completed_cycles(), cycles + 1, "round {round}");
                assert!(!plan.concurrent_work_in_progress());
                assert_eq!(
                    plan.previous_pause(),
                    Some(crate::plan::Pause::FinalMark),
                    "round {round}"
                );
                assert!(memory_manager::is_live_object(a));
                assert!(memory_manager::is_live_object(b));
                assert!(!memory_manager::is_live_object(garbage));
            }
        },
        no_cleanup,
    )
}
