// GITHUB-CI: MMTK_PLAN=ConcurrentSpan

//! The heap limit is hard: a mutator that keeps everything it allocates alive runs out of
//! memory, and the heap never grows past the limit.  An allocation stall used to take its pages
//! after the cycle ended even if the cycle had freed nothing, so the heap grew without bound.

use super::mock_test_prelude::*;
use crate::util::{Address, ObjectReference};
use crate::AllocationSemantics;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// A root slot.  It holds the newest object of a linked list, or 0 for null.
static ROOT: AtomicUsize = AtomicUsize::new(0);

static OUT_OF_MEMORY: AtomicBool = AtomicBool::new(false);

fn field(object: ObjectReference) -> Address {
    object.to_raw_address() + 8usize
}

#[test]
pub fn heap_limit() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                vm_root_slots: MockMethod::new_fixed(Box::new(|_| vec![Address::from_ref(&ROOT)])),
                get_object_size: MockMethod::new_fixed(Box::new(|_| 1024)),
                scan_object: MockMethod::new_fixed(Box::new(|(_, object, visitor)| {
                    visitor.visit_slot(field(object));
                })),
                out_of_memory: MockMethod::new_fixed(Box::new(|_| {
                    OUT_OF_MEMORY.store(true, Ordering::SeqCst);
                })),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            let fixture = MutatorFixture::create_with_heapsize(8 * MB);
            let mmtk = fixture.mmtk();
            let total = mmtk.get_plan().get_total_pages();

            // Allocate a linked list that stays reachable from `ROOT`, up to 4x the heap size.
            let mut allocated = 0;
            while !OUT_OF_MEMORY.load(Ordering::SeqCst) && allocated < 32 * MB {
                let addr = memory_manager::alloc(
                    fixture.mutator(),
                    1024,
                    8,
                    0,
                    AllocationSemantics::Default,
                );
                let reserved = mmtk.get_plan().get_reserved_pages();
                assert!(
                    reserved <= total,
                    "{reserved} pages reserved, over the limit of {total}, after {allocated} bytes"
                );
                if addr.is_zero() {
                    break;
                }
                let object = MockVM::object_start_to_ref(addr);
                memory_manager::post_alloc(
                    fixture.mutator(),
                    object,
                    1024,
                    AllocationSemantics::Default,
                );
                unsafe { field(object).store(ROOT.load(Ordering::SeqCst)) };
                ROOT.store(object.to_raw_address().as_usize(), Ordering::SeqCst);
                allocated += 1024;
            }
            assert!(
                OUT_OF_MEMORY.load(Ordering::SeqCst),
                "No out-of-memory after keeping {allocated} bytes alive in an 8 MB heap"
            );
        },
        no_cleanup,
    )
}
