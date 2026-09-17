// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;

use crate::util::alloc::allocator::AllocationOptions;
use crate::AllocationSemantics;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering;

/// Some bindings never return from `Collection::out_of_memory` (e.g. mmtk-julia longjmps out of
/// it), so any cleanup mmtk-core does after the callback is skipped. This test simulates such a
/// callback with a panic (caught via `catch_unwind`, standing in for the VM's exception handler)
/// and asserts MMTk's OOM bookkeeping is still consistent afterwards.
#[test]
pub fn allocate_oom_unwind_leaves_inconsistent_state() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                // `block_for_gc` is a no-op, but must be implemented: it's called on every failed allocation.
                // We reach the emergency-collection branch by setting `emergency_collection` directly instead,
                // as there's no real GC scheduler here.
                block_for_gc: MockMethod::new_default(),
                // A callback that never returns.
                out_of_memory: MockMethod::new_fixed(Box::new(|(_tls, _err)| {
                    panic!(
                        "Collection::out_of_memory was invoked for a real OOM; a real binding \
                         (e.g. mmtk-julia's jl_throw_out_of_memory_error()) may never return \
                         from this call"
                    );
                })),
                ..MockVM::default()
            }
        },
        || {
            const MB: usize = 1024 * 1024;
            const HEAP: usize = 4 * MB;
            const CHUNK: usize = 4096;

            let mut fixture = MutatorFixture::create_with_heapsize(HEAP);
            let mmtk = fixture.mmtk();

            // The heap is filled with `at_safepoint: false` allocations, which return immediately (success
            // or null) without retrying or blocking for GC.
            let no_retry_options = AllocationOptions {
                at_safepoint: false,
                ..Default::default()
            };
            let mut filled = false;
            for _ in 0..(HEAP / CHUNK + 16) {
                let addr = memory_manager::alloc_with_options(
                    &mut fixture.mutator,
                    CHUNK,
                    8,
                    0,
                    AllocationSemantics::Default,
                    no_retry_options,
                );
                if addr.is_zero() {
                    filled = true;
                    break;
                }
            }
            assert!(
                filled,
                "expected the heap to fill up within the iteration budget"
            );

            // Simulate an emergency collection, as a real GC's bookkeeping would.
            mmtk.state
                .emergency_collection
                .store(true, Ordering::Relaxed);
            mmtk.state
                .allocation_success
                .store(false, Ordering::Relaxed);

            // The heap is full, so this hits the emergency-collection branch and calls
            // `Collection::out_of_memory`, which unwinds rather than returns.
            let panic_result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                memory_manager::alloc(
                    &mut fixture.mutator,
                    CHUNK,
                    8,
                    0,
                    AllocationSemantics::Default,
                )
            }));

            assert!(panic_result.is_err());
            read_mockvm(|mock| {
                assert!(mock.out_of_memory.is_called());
            });

            // `thrown_oom` is only set after the callback returns, so it must still be unset.
            let selector =
                memory_manager::get_allocator_mapping(mmtk, AllocationSemantics::Default);
            let thrown_oom = unsafe { fixture.mutator.allocator(selector) }
                .get_context()
                .thrown_oom
                .load(Ordering::Relaxed);
            assert!(
                !thrown_oom,
                "thrown_oom should not be left set after a failed OOM handling attempt"
            );

            // `allocation_success` must be reset before the callback (which may not return).
            // If it were left `true`, no further allocation could ever throw OOM again (a
            // silent retry livelock).
            assert!(
                !mmtk.state.allocation_success.load(Ordering::Relaxed),
                "allocation_success should have been reset to false after calling OOM"
            );
        },
        no_cleanup,
    )
}
