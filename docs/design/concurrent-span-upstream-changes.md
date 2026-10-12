# ConcurrentSpan: changes to existing mmtk-core code

Bug fixes and improvements to existing mmtk-core code, found or needed while building `ConcurrentSpan`. Each one is (or will be) its own commit on the port branch, so it can be cherry-picked onto `master` and sent upstream on its own. The rules are in [`concurrent-span-engineering.md`](concurrent-span-engineering.md) §2.

Update this file in a separate docs-only commit, never in the change's own commit.

**Status values:** `planned` (identified, no commit yet), `committed` (on the port branch), `upstreamed` (merged into mmtk-core `master`), `dropped` (with a reason).

**Key:** the commit subject is the stable key. The hash is the one at the time of writing; it changes when the branch is rebased.

| ID | Subject | Kind | Area | Status | Commit | Depends on | Notes |
|---|---|---|---|---|---|---|---|
| U1 | Fix nonmoving `MarkSweepSpace` calling `prepare` in release | Bug fix (suspected) | `src/plan/global.rs` | planned | | | `CommonPlan::release_nonmoving_space` calls `self.nonmoving.prepare(_full_heap)` under `marksweep_as_nonmoving` (`src/plan/global.rs:914-915`), where `release` is expected. There is a FIXME nearby, so confirm it is a bug (and add a test) before fixing. Found while writing design §4 |
| U2 | Allow mocking root slots in MockVM | Test infrastructure | `src/util/test_util/mock_vm/` | committed | `89569a86bf` | | `scan_roots_in_mutator_thread` and `scan_vm_specific_roots` only logged a warning, so no MockVM test could have roots. Adds `mutator_root_slots` and `vm_root_slots` mock methods (the scanning methods take a plan-typed factory and cannot be mocked directly). Test: `mock_test_mock_roots`. Needed for phase 4 (concurrent stack scanning) and useful for every plan |

## Details

Longer notes for entries that need them. Use the entry's ID as the heading.
