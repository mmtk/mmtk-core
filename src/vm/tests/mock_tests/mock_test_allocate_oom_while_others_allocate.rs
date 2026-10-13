// GITHUB-CI: MMTK_PLAN=Immix,MarkSweep

//! A mutator whose allocation request cannot be satisfied runs out of memory, even while another
//! mutator keeps allocating successfully.  The other mutator's allocations used to reset the
//! collection attempts, so the request never got an emergency collection or an out-of-memory
//! error: it retried forever.

use super::mock_test_prelude::*;
use crate::global_state::GcStatus;
use crate::util::test_util::mock_vm::mock_api;
use crate::util::{Address, ObjectReference, VMMutatorThread};
use crate::vm::Collection;
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const MB: usize = 1024 * 1024;
const OBJECT_SIZE: usize = 1024;
/// The other mutator gives up after this many mutators have blocked for GCs.  With the fix,
/// the out-of-memory error comes after a few GCs.
const OTHER_LIMIT: usize = 200;

/// A root slot.  It holds the newest object of a linked list kept alive by the test.
static ROOT: AtomicUsize = AtomicUsize::new(0);
static OUT_OF_MEMORY: AtomicBool = AtomicBool::new(false);
/// Set when the other mutator should stop allocating.
static STOP: AtomicBool = AtomicBool::new(false);
static OTHER_STARTED: AtomicBool = AtomicBool::new(false);
/// The number of times a mutator has blocked for a GC.
static BLOCKED: AtomicUsize = AtomicUsize::new(0);
/// Set if the other mutator reached `OTHER_LIMIT` before the out-of-memory error.
static OTHER_GAVE_UP: AtomicBool = AtomicBool::new(false);

fn field(object: ObjectReference) -> Address {
    object.to_raw_address() + 8usize
}

fn safepoint(tls: VMMutatorThread) {
    if mock_api::singleton().state.gc_status.load() == GcStatus::PauseRequested {
        MockVM::block_for_gc(tls);
    }
}

fn alloc_object(tls: VMMutatorThread) -> Option<ObjectReference> {
    let mutator = tls.as_mock_mutator();
    let addr = memory_manager::alloc(mutator, OBJECT_SIZE, 8, 0, AllocationSemantics::Default);
    if addr.is_zero() {
        return None;
    }
    let object = MockVM::object_start_to_ref(addr);
    memory_manager::post_alloc(mutator, object, OBJECT_SIZE, AllocationSemantics::Default);
    unsafe { field(object).store(0usize) };
    Some(object)
}

/// The other mutator: allocates garbage until told to stop.
fn other_mutator() {
    let tls = mock_api::bind_mutator();
    while !STOP.load(Ordering::SeqCst) {
        safepoint(tls);
        alloc_object(tls).expect("small allocations always fit");
        OTHER_STARTED.store(true, Ordering::SeqCst);
        if BLOCKED.load(Ordering::SeqCst) >= OTHER_LIMIT {
            OTHER_GAVE_UP.store(true, Ordering::SeqCst);
            break;
        }
    }
    // Stay a well-behaved mutator until the process exits: a registered mutator that stops
    // polling would block every later pause.
    loop {
        safepoint(tls);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
pub fn allocate_oom_while_others_allocate() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                get_object_size: MockMethod::new_fixed(Box::new(|_| OBJECT_SIZE)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    visitor.visit_slot(field(object));
                })),
                block_for_gc: MockMethod::new_fixed(Box::new(|tls| {
                    BLOCKED.fetch_add(1, Ordering::SeqCst);
                    MUTATOR_PARK.park(tls.0);
                })),
                out_of_memory: MockMethod::new_fixed(Box::new(|_| {
                    OUT_OF_MEMORY.store(true, Ordering::SeqCst);
                })),
                ..MockVM::default()
            }
        },
        || {
            let fixture = MutatorFixture::create_with_heapsize(8 * MB);
            let tls = fixture.mutator_tls();

            // Keep 5 MB alive.
            for _ in 0..5 * MB / OBJECT_SIZE {
                let object = alloc_object(tls).unwrap();
                unsafe { field(object).store(ROOT.load(Ordering::SeqCst)) };
                ROOT.store(object.to_raw_address().as_usize(), Ordering::SeqCst);
            }

            std::thread::spawn(other_mutator);
            while !OTHER_STARTED.load(Ordering::SeqCst) {
                safepoint(tls);
                std::thread::yield_now();
            }

            // 4 MB does not fit next to the 5 MB that stay alive, however often the heap is
            // collected.
            let addr =
                memory_manager::alloc(fixture.mutator(), 4 * MB, 8, 0, AllocationSemantics::Los);
            STOP.store(true, Ordering::SeqCst);
            assert!(addr.is_zero());
            assert!(OUT_OF_MEMORY.load(Ordering::SeqCst));
            assert!(
                !OTHER_GAVE_UP.load(Ordering::SeqCst),
                "No out-of-memory error while another mutator was allocating"
            );
        },
        no_cleanup,
    )
}
