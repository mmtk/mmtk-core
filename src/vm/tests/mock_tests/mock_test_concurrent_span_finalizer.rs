// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Finalizers in a concurrent cycle: the contents of finalizable objects are traced during
//! concurrent marking, and an unreachable finalizable object is resurrected in `FinalMark`
//! together with what it references.

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
pub fn concurrent_finalizer() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                // Scan work counts object sizes.
                get_object_size: MockMethod::new_fixed(Box::new(|_| 48)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    visitor.visit_slot(field(object));
                })),
                schedule_finalization: MockMethod::new_fixed(Box::new(|_| {})),
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

            // ROOT -> a.  `finalizable` -> `child` are unreachable, and `finalizable` has a
            // finalizer.
            let a = alloc(&fixture);
            let finalizable = alloc(&fixture);
            let child = alloc(&fixture);
            let garbage = alloc(&fixture);
            link(finalizable, child);
            ROOT.store(a.to_raw_address().as_usize(), Ordering::SeqCst);
            memory_manager::add_finalizer(mmtk, finalizable);

            FORCE_CONCURRENT_MARKING_FOR_TEST.store(true, Ordering::SeqCst);
            assert!(mmtk.gc_trigger.poll(true, None));
            MockVM::block_for_gc(tls);
            assert!(plan.concurrent_work_in_progress());

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

            for object in [a, finalizable, child] {
                assert!(memory_manager::is_live_object(object), "{object} is dead");
            }
            assert!(!memory_manager::is_live_object(garbage));
            assert_eq!(
                memory_manager::get_finalized_object(mmtk),
                Some(finalizable)
            );
        },
        no_cleanup,
    )
}
