// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! Mark assists: a mutator that allocates while marking is slow goes into debt, parks, and is
//! woken when the GC workers have done enough scan work for it (or marking ends).

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::plan::concurrent::span::ConcurrentSpan;
use crate::plan::ConcurrentPlan;
use crate::util::test_util::mock_vm::mock_api;
use crate::util::{Address, ObjectReference};
use crate::vm::Collection;
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A root slot.  It holds an object reference as a raw address, or 0 for null.
static ROOT: AtomicUsize = AtomicUsize::new(0);

const LIST_LENGTH: usize = 2000;

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
pub fn assists() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                get_object_size: MockMethod::new_fixed(Box::new(|_| 48)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    // Slow marking.
                    std::thread::sleep(std::time::Duration::from_micros(200));
                    visitor.visit_slot(field(object));
                })),
                supports_blocking_mutators: MockMethod::new_fixed(Box::new(|_| true)),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(64 * MB);
            let mmtk = fixture.mmtk();
            let tls = fixture.mutator_tls();

            // A rooted list, which takes a while to mark.
            let head = alloc(&fixture, 48);
            let mut tail = head;
            for _ in 1..LIST_LENGTH {
                let next = alloc(&fixture, 48);
                unsafe { field(tail).store(next) };
                tail = next;
            }
            ROOT.store(head.to_raw_address().as_usize(), Ordering::SeqCst);

            assert!(mmtk.gc_trigger.poll(true, None));
            MockVM::block_for_gc(tls);
            assert!(plan().concurrent_work_in_progress());

            // Allocate garbage while marking.  Refills put the mutator in debt, so it assists.
            let mut allocated = 0;
            while plan().concurrent_work_in_progress() && allocated < 32 * MB {
                alloc(&fixture, 256);
                allocated += 256;
                if mmtk.state.gc_status.load() == GcStatus::PauseRequested {
                    MockVM::block_for_gc(tls);
                }
            }
            for _ in 0..10_000 {
                match mmtk.state.gc_status.load() {
                    GcStatus::PauseRequested => MockVM::block_for_gc(tls),
                    GcStatus::NotInGC if !plan().concurrent_work_in_progress() => break,
                    _ => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            assert!(!plan().concurrent_work_in_progress());

            let (assists, parked) = plan().assist_stats();
            info!("assists: {assists}, parked: {parked}, allocated during marking: {allocated}");
            assert!(parked > 0, "No assist parked");
            let mut object = head;
            for _ in 0..LIST_LENGTH {
                assert!(memory_manager::is_live_object(object));
                object = unsafe { field(object).load::<ObjectReference>() };
            }
        },
        no_cleanup,
    )
}
