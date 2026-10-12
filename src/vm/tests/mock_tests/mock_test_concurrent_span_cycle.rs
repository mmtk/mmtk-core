// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! A concurrent cycle: `InitialMark`, concurrent marking with the hybrid barrier, ragged flush
//! rounds, and `FinalMark`.  Objects form a list from a root, and the test changes the list
//! through the barrier while marking.

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::plan::concurrent::span::global::FORCE_CONCURRENT_MARKING_FOR_TEST;
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::ConcurrentPlan;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A root slot.  It holds an object reference as a raw address, or 0 for null.
static ROOT: AtomicUsize = AtomicUsize::new(0);

/// Every test object has one reference field, one word after the object reference.
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

fn link(from: ObjectReference, to: ObjectReference) {
    unsafe { field(from).store(to) };
}

#[test]
pub fn concurrent_cycle() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    visitor.visit_slot(field(object));
                })),
                supports_concurrent_stack_scanning: MockMethod::new_fixed(Box::new(|_| true)),
                supports_concurrent_mutator_flush: MockMethod::new_fixed(Box::new(|_| true)),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(8 * MB);
            let mmtk = fixture.mmtk();
            let tls = fixture.mutator_tls();
            let plan = mmtk
                .get_plan()
                .downcast_ref::<ConcurrentSpan<MockVM>>()
                .unwrap();

            // ROOT -> a -> b -> c, and `garbage`.
            let a = alloc(&fixture);
            let b = alloc(&fixture);
            let c = alloc(&fixture);
            let garbage = alloc(&fixture);
            link(a, b);
            link(b, c);
            ROOT.store(a.to_raw_address().as_usize(), Ordering::SeqCst);

            // Start a cycle, and wait for the `InitialMark` pause.
            FORCE_CONCURRENT_MARKING_FOR_TEST.store(true, Ordering::SeqCst);
            assert!(mmtk.gc_trigger.poll(true, None));
            MockVM::block_for_gc(tls);
            assert!(plan.concurrent_work_in_progress());

            // While marking: a -> c (the barrier logs b and c), then b -> null (logs c).
            memory_manager::object_reference_write_pre(fixture.mutator(), a, field(a), Some(c));
            link(a, c);
            memory_manager::object_reference_write_pre(fixture.mutator(), b, field(b), None);
            unsafe { field(b).store(0usize) };

            // Park whenever a pause is requested, until the cycle is over.
            let mut finished = false;
            for _ in 0..10_000 {
                match mmtk.state.gc_status.load() {
                    GcStatus::PauseRequested => MockVM::block_for_gc(tls),
                    GcStatus::NotInGC if !plan.concurrent_work_in_progress() => {
                        finished = true;
                        break;
                    }
                    _ => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            assert!(finished, "The concurrent cycle did not finish");
            let (ragged_rounds, restarts) = plan.cycle_stats();
            info!("ragged rounds: {ragged_rounds}, restarts: {restarts}");
            assert!(ragged_rounds >= 1);

            for object in [a, b, c] {
                assert!(memory_manager::is_live_object(object), "{object} is dead");
            }
            assert!(!memory_manager::is_live_object(garbage));
            assert_eq!(
                unsafe { memory_manager::barrier_active_address().load::<u8>() },
                0
            );
        },
        no_cleanup,
    )
}
