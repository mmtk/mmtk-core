# ConcurrentSpan: changes to existing mmtk-core code

Bug fixes and improvements to existing mmtk-core code, found or needed while building `ConcurrentSpan`. Each one is (or will be) its own commit on the port branch, so it can be cherry-picked onto `master` and sent upstream on its own. The rules are in [`concurrent-span-engineering.md`](concurrent-span-engineering.md) §2.

Update this file in a separate docs-only commit, never in the change's own commit.

**Status values:** `planned` (identified, no commit yet), `committed` (on the port branch), `upstreamed` (merged into mmtk-core `master`), `dropped` (with a reason).

**Key:** the commit subject is the stable key. The hash is the one at the time of writing; it changes when the branch is rebased.

| ID | Subject | Kind | Area | Status | Commit | Depends on | Notes |
|---|---|---|---|---|---|---|---|
| U1 | Fix nonmoving `MarkSweepSpace` calling `prepare` in release | Bug fix (suspected) | `src/plan/global.rs` | planned | | | `CommonPlan::release_nonmoving_space` calls `self.nonmoving.prepare(_full_heap)` under `marksweep_as_nonmoving` (`src/plan/global.rs:914-915`), where `release` is expected. There is a FIXME nearby, so confirm it is a bug (and add a test) before fixing. Found while writing design §4 |
| U2 | Allow mocking root slots in MockVM | Test infrastructure | `src/util/test_util/mock_vm/` | committed | `89569a86bf` | | `scan_roots_in_mutator_thread` and `scan_vm_specific_roots` only logged a warning, so no MockVM test could have roots. Adds `mutator_root_slots` and `vm_root_slots` mock methods (the scanning methods take a plan-typed factory and cannot be mocked directly). Test: `mock_test_mock_roots`. Needed for phase 4 (concurrent stack scanning) and useful for every plan |
| U3 | Let `on_concurrent_work_drained` ask the last parked worker to look again | API improvement | `src/plan/concurrent/global.rs`, `src/scheduler/scheduler.rs` | planned | | | Returns `bool`. Currently inside the port commit "ConcurrentSpan: concurrent marking with Go-style mark termination"; needs splitting out |
| U4 | Add `Scanning::scan_roots_in_mutator_threads_concurrently`, `Collection::flush_mutators_concurrently` and `ConcurrentScanStacks` | API addition (MEP) | `src/vm/`, `src/scheduler/gc_work.rs` | planned | | | Same port commit as U3. Defaults keep current behaviour |
| U5 | Allow more than `concurrent_threads` active workers when a pause starts | Bug fix (assertion) | `src/scheduler/scheduler.rs` | planned | | | Inside "mark assists"; any plan that activates extra workers during concurrent work trips the old assertion |
| U6 | A pause that closes a concurrent cycle is not a new collection attempt | Bug fix | `src/scheduler/gc_work.rs` | planned | | | Inside "Fix false out-of-memory after an allocation stall". Also changes `ConcurrentImmix`'s FinalMark |
| U7 | `GCTrigger::poll_or_stall` and `ConcurrentPlan::stall_instead_of_pause` | API addition | `src/util/heap/gc_trigger.rs`, `src/policy/space.rs` | planned | | | Allocation stalls for any concurrent plan. After a stall, `Space::acquire` polls again instead of taking the pages: taking them unconditionally let the heap grow past its limit when the cycle freed nothing (fixed in "Keep the heap limit hard after an allocation stall") |
| U8 | `Collection::block_mutator_until` / `wake_blocked_mutators` | API addition | `src/vm/collection.rs` | planned | | | Used by assists |
| U9 | Map64: fail chunk allocation when the free list cannot grow | Bug fix | `src/util/heap/layout/map64.rs` | committed | `32b68dbbbf` | | Found by `mock_test_concurrent_span_stress` with a 4 MiB heap: index out of bounds in `RawMemoryFreeList` |
| U10 | MockVM: report which threads a GC is waiting for | Test infrastructure | `src/util/test_util/mock_vm/thread_park.rs` | committed | `936c32a839` | | Diagnostic for hanging multi-threaded mock tests |
| U11 | Count an allocation request that keeps failing as a collection attempt | Bug fix | `src/global_state.rs`, `src/util/alloc/allocator.rs` | committed | `8b368ddd6e` | | A request that cannot be satisfied (a large object that does not fit) never got an emergency collection or an out-of-memory error while other mutators kept allocating, because any successful allocation reset the collection attempts. Found with ConcurrentSpan on cassandra (decision D93), where it livelocked until the benchmark timeout. Test: `mock_test_allocate_oom_while_others_allocate` (fails without the fix with MarkSweep and ConcurrentSpan; Immix passes either way) |

## Details

Longer notes for entries that need them. Use the entry's ID as the heading.
