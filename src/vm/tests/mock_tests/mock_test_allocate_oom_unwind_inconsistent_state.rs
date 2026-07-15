// GITHUB-CI: MMTK_PLAN=all

use super::mock_test_prelude::*;

use crate::util::alloc::allocator::AllocationOptions;
use crate::AllocationSemantics;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering;

/// `alloc_slow_inline`'s emergency-collection branch (allocator.rs:559-580) swaps
/// `GlobalState::allocation_success` to `true` *before* calling `self.out_of_memory(tls)`, then
/// resets it back to `false` right after. Some bindings never return from that call (e.g.
/// mmtk-julia's `jl_throw_out_of_memory_error()` is a C `longjmp`), skipping the reset. This test
/// simulates that with an assertion failure inside the callback, caught via `catch_unwind`
/// further up (standing in for the VM's own exception handler), then checks whether MMTk's state
/// was left consistent.
///
/// `block_for_gc` is a no-op (never frees memory): it's not how we reach the emergency branch
/// (we set `GlobalState::emergency_collection` directly, since there's no real GC scheduler
/// here), but it's still called on every failed allocation, so it must not be left `unimplemented`.
///
/// The heap is filled with `at_safepoint: false` allocations, which always return immediately
/// (success or null, never retrying or calling `block_for_gc` -- see the `!at_safepoint` early
/// returns in `alloc_slow_inline` and `Space::not_acquiring`). This keeps the test plan-agnostic
/// and hang-safe, regardless of each plan's allocator type or usable heap fraction.
#[test]
pub fn allocate_oom_unwind_leaves_inconsistent_state() {
    with_mockvm(
        || -> MockVM {
            MockVM {
                block_for_gc: MockMethod::new_default(),
                // Simulates a callback that never returns (see the doc comment above).
                out_of_memory: MockMethod::new_fixed(Box::new(|(_tls, _err)| {
                    assert!(
                        false,
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

            // Fill the heap with small, non-retrying allocations until one genuinely fails.
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

            // Simulate an emergency collection (see the doc comment above) and reset
            // `allocation_success` for the new epoch, as a real GC's bookkeeping would.
            mmtk.state.emergency_collection.store(true, Ordering::Relaxed);
            mmtk.state.allocation_success.store(false, Ordering::Relaxed);

            // The heap is already full, so this retries into the emergency-collection branch and
            // calls `Collection::out_of_memory`, which we've set up to unwind rather than return.
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

            // Unlike `allocation_success`, `thrown_oom` is only set *after* the callback returns
            // (allocator.rs:343), so it's untouched here rather than corrupted -- worth asserting
            // explicitly to document that contrast.
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

            // BUG: the reset at allocator.rs:573-577 never ran, so `allocation_success` is stuck
            // `true` -- as if an allocation had succeeded since the last emergency collection.
            // This breaks the heuristic (`GlobalState::determine_collection_attempts`) that
            // decides when to declare a genuine OOM: any further unsatisfiable allocation would
            // find `allocation_success` already `true` and silently retry forever instead (a
            // livelock) -- which is why this test doesn't itself attempt a further allocation.
            assert!(
                !mmtk.state.allocation_success.load(Ordering::Relaxed),
                "allocation_success should have been reset to false after handling OOM, but it \
                 is stuck at true because Collection::out_of_memory did not return normally. \
                 MMTk's OOM bookkeeping is now inconsistent."
            );
        },
        no_cleanup,
    )
}
