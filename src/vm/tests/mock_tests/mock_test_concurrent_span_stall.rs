// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! An allocation stall: the heap fills up during concurrent marking, and the allocating mutator
//! waits for the cycle to end instead of interrupting it with a stop-the-world pause.

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::plan::concurrent::span::global::FORCE_CONCURRENT_MARKING_FOR_TEST;
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::ConcurrentPlan;
use crate::util::test_util::mock_vm::mock_api;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A root slot.  It holds an object reference as a raw address, or 0 for null.
static ROOT: AtomicUsize = AtomicUsize::new(0);

fn field(object: ObjectReference) -> Address {
    object.to_raw_address() + 8usize
}

fn plan() -> &'static ConcurrentSpan<MockVM> {
    mock_api::singleton()
        .get_plan()
        .downcast_ref::<ConcurrentSpan<MockVM>>()
        .unwrap()
}

fn alloc(fixture: &MutatorFixture, size: usize) -> ObjectReference {
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
pub fn allocation_stall() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    // Hold up marking at the rooted object until a mutator has stalled, so that
                    // marking cannot finish before the heap is full.
                    if object.to_raw_address().as_usize() == ROOT.load(Ordering::SeqCst) {
                        for _ in 0..20_000 {
                            if plan().stall_count() > 0 {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                    }
                    visitor.visit_slot(field(object));
                })),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(8 * MB);
            let mmtk = fixture.mmtk();
            let tls = fixture.mutator_tls();

            let a = alloc(&fixture, 48);
            let b = alloc(&fixture, 48);
            unsafe { field(a).store(b) };
            ROOT.store(a.to_raw_address().as_usize(), Ordering::SeqCst);

            FORCE_CONCURRENT_MARKING_FOR_TEST.store(true, Ordering::SeqCst);
            assert!(mmtk.gc_trigger.poll(true, None));
            MockVM::block_for_gc(tls);
            assert!(plan().concurrent_work_in_progress());

            // Allocate garbage until the heap is full.  The allocation that does not fit stalls
            // until `FinalMark` has freed memory, and then succeeds.  (On this space, objects
            // allocated during marking are not black, so they are garbage anyway.)
            let mut allocated = 0;
            while plan().stall_count() == 0 && allocated < 64 * MB {
                alloc(&fixture, 1024);
                allocated += 1024;
            }
            assert!(plan().stall_count() > 0, "No allocation stalled");
            assert!(allocated > 4 * MB, "Stalled after only {allocated} bytes");

            // The stalled allocation returned after the cycle ended.
            for _ in 0..10_000 {
                match mmtk.state.gc_status.load() {
                    GcStatus::PauseRequested => MockVM::block_for_gc(tls),
                    GcStatus::NotInGC if !plan().concurrent_work_in_progress() => break,
                    _ => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            assert!(!plan().concurrent_work_in_progress());
            assert!(memory_manager::is_live_object(a));
            assert!(memory_manager::is_live_object(b));
        },
        no_cleanup,
    )
}
