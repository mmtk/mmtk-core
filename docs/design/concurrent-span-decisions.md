# ConcurrentSpan: decisions for review

Decisions made without asking the project lead, so that work could continue. Each one records the question, the options, what was chosen and why, and how to undo it. The rules are in [`concurrent-span-engineering.md`](concurrent-span-engineering.md) §5.

**For the reviewer:** set the status to `accepted`, or to `changed` with a note saying what you want instead. A `changed` decision is then reworked and the outcome recorded under its entry.

**Status values:** `for review`, `accepted`, `changed`, `open` (something that must not be done autonomously, such as pushing, and is waiting for you).

## Summary

| ID | Date | Question | Decision | Easy to undo? | Status |
|---|---|---|---|---|---|
| D1 | 2026-10-12 | Where do the engineering documents live? | Flat files next to the design doc in `docs/design/`, prefixed `concurrent-span-` | Yes | for review |
| D2 | 2026-10-12 | `define_mock_vm!` (configurable MockVM) is on the `configurable-mock-vm` branch, not on `master`. Do we build on it? | Not at first. Use the default MockVM. If a test needs a different config, cherry-pick that branch's MockVM change onto this branch as a separable commit, and record it here and in the upstream-changes doc | Yes | for review |
| D3 | 2026-10-12 | How are fixes to existing code kept separable? | One commit per fix, made before the port commits that use it, with the upstream-changes doc updated in a separate docs-only commit | Yes | for review |
| D4 | 2026-10-12 | Commit the design and engineering docs? | Yes: one docs-only commit on `mmtk-core-go-gc` with the design doc and these four documents | Yes (amend or revert locally; nothing is pushed) | for review |
| D5 | 2026-10-12 | Which memory caps, and how to enforce them? | cgroup caps through `systemd-run --user --scope`: 8 GB for builds, 4 GB per MockVM test, `-Xmx` + 2 GB per JVM; at most 16 GB at once | Yes | for review |
| D6 | 2026-10-12 | Scope of the first 12-hour session | Breadth over polish (project lead's answer): go as far as possible through the phases, including phase 4, with less OpenJDK validation per phase. Agents work in parallel git worktrees | Yes | accepted |
| D7 | 2026-10-12 | Which OpenJDK tree to build | A new fastdebug build in `~/Code/openjdk` (project lead's answer). The existing builds in `~/openjdk` are left alone | Yes | accepted |
| D8 | 2026-10-12 | The development machine has 8 cores and 125 GB, not 24 cores and 31 GB as engineering §6 says | Keep the 16 GB total. With up to three parallel agents, each build or test uses `MemoryMax=5G` and `-j 3` | Yes | for review |
| D30 | 2026-10-12 | Which page resource and unit for `SpanSpace` (design open question 1)? | `FreeListPageResource`, acquiring 128 KiB page regions through `Space::acquire`; empty regions go back to MMTk at once | Yes | for review |
| D31 | 2026-10-12 | `GcBits` arenas: side-metadata mmapper or plain `mmap` (design open question 2)? | Neither: fixed per-span bitmaps inline in the span descriptor (plain Rust heap memory); sweep copies marks to alloc bits | Yes | for review |
| D32 | 2026-10-12 | Lock-free span sets for the central lists? | Not yet: one mutex per span class | Yes | for review |
| D33 | 2026-10-12 | Add `AllocationSemantics::NoScan` now? | Deferred: it is a public API change (MEP) touching every plan and binding. Noscan span classes and `SpanAllocator::set_noscan` are ready for it | Yes | for review |
| D34 | 2026-10-12 | How to expose the span allocator to bindings? | `AllocatorSelector::Span`, two span allocators per mutator (scan, noscan), and `AllocatorInfo::Span` with the offsets of the span array and of the span descriptor fields | Yes | for review |
| D35 | 2026-10-12 | How does the STW (phase 1-2) plan sweep? | Go's way: `sweepgen += 2` in Release, mutator release uncaches (and sweeps stale) spans, `SweepSpans` packets in the `Release` bucket, lazy sweeping by allocators for anything left | Yes | for review |
| D36 | 2026-10-12 | IDs for SpanSpace deviations, while other agents edit the same file | Prefix `SP` in a separate table | Yes | for review |
| D37 | 2026-10-12 | How to run Go's size-class generator without a Go install or network module downloads | `tools/spanspace-sizeclasses/gen.sh` compiles Go's unmodified `mksizeclasses.go` and `constants.go` with a small Rust emitter (`emit_rust.go`), with any Go toolchain; the official go1.25.1 tarball was used, outside the repository | Yes | for review |
| D38 | 2026-10-12 | What does `is_live` mean for a `SpanSpace` object outside marking? | During marking: the mark bit. Afterwards: the mark bit for unswept spans and the allocation state for swept spans | Yes | for review |

## D1. Location of the engineering documents

**Question.** Where should the engineering, upstream-changes, deviations and decisions documents live?

**Options.**
1. Flat files next to `docs/design/go-gc-port.md`, with a `concurrent-span-` prefix.
2. A `docs/design/concurrent-span/` folder, with the design doc moved into it.
3. Outside the repository.

**Decision.** Option 1. The design doc stays where it is (it was already open in the editor), and the prefix groups the files.

**To undo.** Move the files with `git mv` and fix the links in the five documents.

## D2. Configurable MockVM

**Question.** `ConcurrentSpan` tests may need MockVM configurations other than the default (for example side metadata). `define_mock_vm!` provides that, but it is on the `configurable-mock-vm` branch, which is not merged into `master`. Do we base the port on it?

**Options.**
1. Base the port branch on `configurable-mock-vm`.
2. Use the default MockVM, and cherry-pick the configurable-MockVM change only when a test needs it.
3. Reimplement a configuration mechanism on this branch.

**Decision.** Option 2. Phase 1 and 2 tests do not need a special configuration. Basing the branch on an unmerged branch would make every port commit depend on it, and reimplementing it would duplicate work.

**To undo.** Rebase the port branch onto `configurable-mock-vm` (option 1).

## D3. Keeping fixes separable

**Question.** The instruction is to make bug fixes and improvements separate commits and to keep a list of them. How exactly?

**Decision.**
- One commit per fix or improvement, containing nothing else, and applicable to `master` on its own.
- Made before the port commits that use it.
- The upstream-changes doc is updated in a separate docs-only commit, so cherry-picking the fix does not bring the doc along.
- The commit subject is the stable key in the doc, because hashes change on rebase.

**To undo.** Change engineering §2. Existing commits are unaffected.

## D4. Committing the documents

**Question.** The design doc and the new engineering documents are untracked. Commit them?

**Decision.** Yes, as one docs-only commit on the `mmtk-core-go-gc` branch, which is the port branch in this worktree. Commits are allowed; pushing is not, so this stays local.

**To undo.** `git reset --soft HEAD~1` (or amend). Nothing has left the machine.

## D5. Memory caps

**Question.** Every process must have bounded memory. What mechanism, and what limits?

**Options for the mechanism.**
1. `ulimit -v`: fails for MMTk, which reserves a large virtual address range up front.
2. `prlimit --rss`: not enforced by Linux.
3. A cgroup memory limit through `systemd-run --user --scope -p MemoryMax=...`: limits resident memory for the command and everything it starts. It works on the development machine (a process using 400 MB under a 200 MB cap was killed).

**Decision.** Option 3, with these limits:
- builds, clippy and unit tests: 8 GB, with `-j 8`;
- one MockVM test: 4 GB, with a fixed heap of normally 1 GB or less;
- one OpenJDK run: `-Xmx` + 2 GB;
- everything running at once: at most 16 GB, half of the machine's 31 GB.

The limits are guesses with headroom. If a legitimate build or run is killed, raise that cap and record the new value here.

**To undo.** Change engineering §6.

## D30. Page resource and unit for `SpanSpace`

**Question.** Design open question 1: which page resource hands out the memory `SpanSpace` carves into pages, and in what unit? The design suggested 4 MiB chunks via `BlockPageResource` with a chunk-sized block, or an aligned `FreeListPageResource`.

**Options.**
1. `BlockPageResource` with a chunk-sized block. Its `release_block` pushes to a per-GC-worker queue (`current_worker_ordinal().unwrap()`), so a mutator that sweeps lazily and frees a span's last pages could not release the block.
2. `FreeListPageResource`, 4 MiB units. MMTk's free list keeps equal-sized requests aligned, so units are chunk-aligned. But a 4 MiB unit is larger than the 1 MiB heaps many MockVM tests use, and makes MMTk's heap accounting very coarse.
3. `FreeListPageResource`, 128 KiB units (16 SpanSpace pages, enough for the largest 10-page span). Also naturally aligned.

**Decision.** Option 3. The page allocator keeps a bitmap per region and does address-ordered first fit over regions (Go's `pageAlloc` without the radix tree). A region whose pages are all free is returned to MMTk immediately, so that MMTk's `FixedHeapSize` accounting and other spaces see it; Go keeps the memory and relies on the scavenger. Recorded as deviation SP2. The unit is one constant (`LOG_BYTES_IN_PAGE_REGION` in `src/policy/spanspace/pages.rs`).

**To undo.** Change the constant (any power of two from 128 KiB to 4 MiB works), or keep empty regions in the page allocator instead of releasing them (`SpanSpace::free_span`).

## D31. Mark and alloc bitmaps (`GcBits`)

**Question.** Design open question 2: should `GcBits` arenas use MMTk's side-metadata mmapper or plain `mmap`?

**Decision.** No arenas. Each span descriptor holds two fixed 1024-bit bitmaps (Go's maximum objects per span). Sweeping copies the mark bits into the alloc bits and zeroes the mark bits, which keeps Go's invariant (marks are cleared per span by sweeping, never at Prepare, so lazy sweeping is safe with concurrent marking). The bitmaps are ordinary Rust heap memory, not counted in MMTk's heap accounting (like Go's GC metadata, which is not counted as heap). Recorded as deviation SP1. The project lead's guidance was to prefer simpler mechanisms here.

**To undo.** Replace the inline arrays with pointers into epoch arenas, and make sweep swap pointers.

## D32. Central span sets

**Decision.** A mutex per span class protecting `Vec`s, as the task allowed. Deviation SP3.

**To undo.** Port `runtime/mspanset.go`.

## D33. `AllocationSemantics::NoScan`

**Question.** The design proposes a new public `AllocationSemantics::NoScan` (an MEP item), mapped to `Default` in all other plans.

**Decision.** Deferred. Adding a variant to a public `#[repr]` enum affects every plan's allocator mapping and every binding that matches on it, and mmtk-openjdk would not use it (design §5.3). The policy side is ready: span classes have a noscan bit, `trace_object` does not enqueue objects in noscan spans, and `SpanAllocator::set_noscan(true)` makes an allocator use noscan spans. Two span allocators are reserved per mutator for this.

**To undo.** Add the variant, map it to `Default` in `create_allocator_mapping`, and to `AllocatorSelector::Span(1)` (with `set_noscan(true)`) in `ConcurrentSpan`.

## D34. `AllocatorSelector::Span` and `AllocatorInfo::Span`

**Decision.** A new `AllocatorSelector::Span(u8)`, `MAX_SPAN_ALLOCATORS = 2`, and `AllocatorInfo::Span { spans_offset, alloc_cache_offset, freeindex_offset, nelems_offset, elemsize_offset, start_offset, alloc_count_offset, needzero_offset }`. `spans_offset` is the offset from the mutator to the allocator's per-span-class array of span pointers; the other fields are offsets in the `#[repr(C)]` span descriptor. A binding computes the span class from the size (Go's tables are public in `policy::spanspace::sizeclasses`). This changes the size of `Allocators`, and so the `Mutator` layout that bindings mirror.

**To undo.** Remove the variant and fall back to `AllocatorInfo::Unimplemented`.

## D35. Sweeping in the stop-the-world phase

**Decision.** In `Pause::Full`, Prepare calls `SpanSpace::prepare_initial_mark` (sweep termination) and Release calls `release_after_mark` (`sweepgen += 2`) and `schedule_sweep(WorkBucketStage::Release)`. Mutator release (after the plan's release) uncaches each mutator's spans and sweeps the stale ones. Whatever the packets leave is swept lazily by allocators. When concurrent marking lands, the plan can move the packets to the `Concurrent` bucket. With the `vo_bit` feature, sweeping must stay in the pause, because dead objects' VO bits must be cleared by the end of the GC.

**To undo.** Schedule the packets elsewhere, or not at all (pure lazy sweeping, with the residue swept in `prepare_initial_mark`).

## D36. Deviation IDs for SpanSpace

**Decision.** SpanSpace entries in the deviations document use the prefix `SP`, in their own table, so that agents working in parallel on the same document do not collide on IDs.

## D37. Running Go's size-class generator

**Question.** There is no Go toolchain installed, and Go's `_mkmalloc` module depends on `golang.org/x/tools`.

**Decision.** `tools/spanspace-sizeclasses/gen.sh` copies Go's `mksizeclasses.go` and `constants.go` (unmodified) into a temporary module with `emit_rust.go`, which calls Go's own `makeClasses`/`computeDivMagic`/`printComment` and prints Rust. It needs no module downloads. The checked-in `src/policy/spanspace/sizeclasses.rs` was generated with the official go1.25.1 linux-amd64 toolchain (downloaded to a scratch directory, not into the repository) from Go `2c3c1caad2`. The output's tables match Go's checked-in `internal/runtime/gc/sizeclasses.go`.

**To undo.** Delete the tool and hand-maintain the table (not recommended: engineering §3 asks for generated tables).

## D38. `is_live` outside marking

**Question.** MMTk's `is_live` is mostly asked during GC, but bindings can ask at other times. Go's equivalent depends on the sweep state.

**Decision.** While marking (from `prepare_initial_mark` to `release_after_mark`): the mark bit. Afterwards, per span: unswept (or stale cached) spans answer from the mark bit, swept spans from the allocation state (`index < freeindex` or the alloc bit).

**To undo.** Change `SFT::is_live` in `src/policy/spanspace/global.rs`.
