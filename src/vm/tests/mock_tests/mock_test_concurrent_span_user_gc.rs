// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! A user-requested GC during concurrent marking.  The request interrupts marking, and
//! `FinalMark` finishes it in the pause, including concurrent packets that had not run yet (the
//! concurrent stack scan, ragged flushes).  Those must not use the VM's concurrent mechanisms in
//! the pause (MockVM asserts that), and objects reachable only from a mutator's stack must
//! survive.

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::ConcurrentPlan;
use crate::util::test_util::mock_vm::mock_api;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A VM root and a stack root.  Raw object addresses, 0 for null.
static VM_ROOT: AtomicUsize = AtomicUsize::new(0);
static STACK_ROOT: AtomicUsize = AtomicUsize::new(0);
/// Marking is held up until the user GC has been requested.
static HOLD_UP_MARKING: AtomicBool = AtomicBool::new(true);

fn field(object: ObjectReference) -> Address {
    object.to_raw_address() + 8usize
}

fn plan() -> &'static ConcurrentSpan<MockVM> {
    mock_api::singleton()
        .get_plan()
        .downcast_ref::<ConcurrentSpan<MockVM>>()
        .unwrap()
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
pub fn user_gc_during_marking() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| {
                    vec![Address::from_ref(&VM_ROOT)]
                })),
                mutator_root_slots: MockMethod::new_fixed(Box::new(|_| {
                    vec![Address::from_ref(&STACK_ROOT)]
                })),
                get_object_size: MockMethod::new_fixed(Box::new(|_| 48)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    for _ in 0..10_000 {
                        if !HOLD_UP_MARKING.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    visitor.visit_slot(field(object));
                })),
                supports_concurrent_stack_scanning: MockMethod::new_fixed(Box::new(|_| true)),
                supports_concurrent_mutator_flush: MockMethod::new_fixed(Box::new(|_| true)),
                ..MockVM::default()
            }
        },
        || {
            // One concurrent worker, so packets are still queued when the user GC arrives.
            let fixture = MutatorFixture::create_with_builder(|builder| {
                builder.options.gc_trigger.set(
                    crate::util::options::GCTriggerSelector::FixedHeapSize(8 << 20),
                );
                builder.options.concurrent_threads.set(1);
            });
            let mmtk = fixture.mmtk();
            let tls = fixture.mutator_tls();

            let a = alloc(&fixture);
            let b = alloc(&fixture);
            let on_stack = alloc(&fixture);
            let child = alloc(&fixture);
            unsafe { field(a).store(b) };
            unsafe { field(on_stack).store(child) };
            VM_ROOT.store(a.to_raw_address().as_usize(), Ordering::SeqCst);
            STACK_ROOT.store(on_stack.to_raw_address().as_usize(), Ordering::SeqCst);

            assert!(mmtk.gc_trigger.poll(true, None));
            MockVM::block_for_gc(tls);
            assert!(plan().concurrent_work_in_progress());

            // A user GC while marking is held up.  Release marking once the pause is requested.
            let releaser = std::thread::spawn(move || {
                while mock_api::singleton().state.gc_status.load() != GcStatus::PauseRequested
                    && mock_api::singleton().state.gc_status.load() != GcStatus::InPause
                {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                HOLD_UP_MARKING.store(false, Ordering::SeqCst);
            });
            memory_manager::handle_user_collection_request(mmtk, tls, true);
            releaser.join().unwrap();

            for _ in 0..10_000 {
                match mmtk.state.gc_status.load() {
                    GcStatus::PauseRequested => MockVM::block_for_gc(tls),
                    GcStatus::NotInGC if !plan().concurrent_work_in_progress() => break,
                    _ => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            assert!(!plan().concurrent_work_in_progress());
            for object in [a, b, on_stack, child] {
                assert!(memory_manager::is_live_object(object), "{object} is dead");
            }
        },
        no_cleanup,
    )
}
