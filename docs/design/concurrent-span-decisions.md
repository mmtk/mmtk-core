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
| D9 | 2026-10-12 | Where does the pacer plug in: a new `GCTriggerSelector::FeedbackPacer` (design §5.10) or the plan? | The plan: `ConcurrentSpan::collection_required` asks the pacer; the GC trigger's heap size stays the hard limit and is the default memory limit | Yes | for review |
| D10 | 2026-10-12 | How does a mutator stall (design §5.11) without new binding APIs? | A `ConcurrentPlan::stall_instead_of_pause` hook: `GCTrigger::poll_or_stall` reports a stall, and `Space::acquire` blocks in `block_for_gc` until the next pause (FinalMark) ends, then takes its pages | Yes | for review |
| D11 | 2026-10-12 | Is `System.gc()` a concurrent cycle that blocks the caller (design §5.2)? | No: still a stop-the-world `Pause::Full`, as in `ConcurrentImmix`. Waiting for a concurrent cycle with `block_for_gc` is racy (the cycle may end between the check and the wait) | Yes | for review |
| D12 | 2026-10-12 | How many times may FinalMark restart concurrent marking? | At most `concurrent_span_max_mark_restarts` (8); then FinalMark finishes the marking in the pause. Go has no limit | Yes | for review |
| D13 | 2026-10-12 | Where do ragged flush rounds run? | As a `RaggedFlush` packet in `Concurrent`, not in the last-parked-worker callback, which holds the worker monitor lock: a handshake there can deadlock with a mutator requesting a GC. Without `flush_mutators_concurrently`, FinalMark is requested directly | Yes | for review |
| D14 | 2026-10-12 | `ConcurrentPlan::on_concurrent_work_drained` must be able to add work or ask for a pause | It returns `bool`; `true` makes the last parked worker look again. Changes `ConcurrentImmix` and LXR (return `false`). Candidate upstream change, now inside a port commit | Yes | for review |
| D15 | 2026-10-12 | Soft references and finalizers make FinalMark trace (design §5.14) | Soft referents are strong in non-emergency cycles (`memory_manager::soft_references_are_strong`, used by mmtk-openjdk); finalizable objects' contents are traced while marking (`ScanFinalizableContents`) | Yes | for review |
| D16 | 2026-10-12 | Does a pause that closes a concurrent cycle count as a collection attempt for out-of-memory? | No. FinalMark continues the attempt started by InitialMark; otherwise a stalled mutator saw a false emergency and OOM | Yes | for review |
| D17 | 2026-10-12 | How are assists parked (design §5.11 Stage A)? | New optional `Collection::block_mutator_until` / `wake_blocked_mutators`; mmtk-openjdk blocks the thread in the VM and waits on `gc_lock` (1 ms timed waits) | Yes | for review |
| D18 | 2026-10-12 | Weak→strong block during mark termination (design §5.14) | Not implemented yet. Weak loads only shade while marking, as with SATB; termination can take extra rounds or restarts | — | open |
| D19 | 2026-10-12 | Work split between the integrator and agents in this session | Pacer (D20–D23) and SpanSpace (D30–D38) by agents in worktrees, merged by the integrator; mmtk-openjdk `HybridBarrier` (D50–D58) by an agent in the shared checkout | — | for review |
| D20 | 2026-10-12 | Which `defaultHeapMinimum` (Go has 4 MiB, or 512 KiB under the `HeapMinimum512KiB` experiment)? | 4 MiB, Go's default | Yes | for review |
| D21 | 2026-10-12 | How is the pacer shared between mutators, workers and pauses? | All fields atomic (`Relaxed`), all methods `&self`; STW-only methods document their precondition | Yes | for review |
| D22 | 2026-10-12 | Go's `math/rand` source is not in the reference checkout. How are `TestGcPacer`'s random streams ported? | SplitMix64 with the same seeds and amplitudes | Yes | for review |
| D23 | 2026-10-12 | The pacer agent's worktree branch was created from `master`, not `mmtk-core-go-gc`, and a hard reset was refused | Work on a new branch `concurrent-span-pacer` created from `mmtk-core-go-gc` in the same worktree; the original worktree branch is left untouched | Yes | for review |
| D30 | 2026-10-12 | Which page resource and unit for `SpanSpace` (design open question 1)? | `FreeListPageResource`, acquiring 128 KiB page regions through `Space::acquire`; empty regions go back to MMTk at once | Yes | for review |
| D31 | 2026-10-12 | `GcBits` arenas: side-metadata mmapper or plain `mmap` (design open question 2)? | Neither: fixed per-span bitmaps inline in the span descriptor (plain Rust heap memory); sweep copies marks to alloc bits | Yes | for review |
| D32 | 2026-10-12 | Lock-free span sets for the central lists? | Not yet: one mutex per span class | Yes | for review |
| D33 | 2026-10-12 | Add `AllocationSemantics::NoScan` now? | Deferred: it is a public API change (MEP) touching every plan and binding. Noscan span classes and `SpanAllocator::set_noscan` are ready for it | Yes | for review |
| D34 | 2026-10-12 | How to expose the span allocator to bindings? | `AllocatorSelector::Span`, two span allocators per mutator (scan, noscan), and `AllocatorInfo::Span` with the offsets of the span array and of the span descriptor fields | Yes | for review |
| D35 | 2026-10-12 | How does the STW (phase 1-2) plan sweep? | Go's way: `sweepgen += 2` in Release, mutator release uncaches (and sweeps stale) spans, `SweepSpans` packets in the `Release` bucket, lazy sweeping by allocators for anything left | Yes | for review |
| D36 | 2026-10-12 | IDs for SpanSpace deviations, while other agents edit the same file | Prefix `SP` in a separate table | Yes | for review |
| D37 | 2026-10-12 | How to run Go's size-class generator without a Go install or network module downloads | `tools/spanspace-sizeclasses/gen.sh` compiles Go's unmodified `mksizeclasses.go` and `constants.go` with a small Rust emitter (`emit_rust.go`), with any Go toolchain; the official go1.25.1 tarball was used, outside the repository | Yes | for review |
| D38 | 2026-10-12 | What does `is_live` mean for a `SpanSpace` object outside marking? | During marking: the mark bit. Afterwards: the mark bit for unswept spans and the allocation state for swept spans | Yes | for review |
| D50 | 2026-10-12 | Which flag does compiled code test for `HybridBarrier`? | mmtk-core's own flag, through `memory_manager::barrier_active_address()`, exported to C++ as `mmtk_barrier_active_address()` and embedded as a constant. Not the binding's `CONCURRENT_MARKING_ACTIVE` | Yes | for review |
| D51 | 2026-10-12 | C2 elides barriers on stores into just-allocated objects (SATB relies on this). Keep it for `HybridBarrier`? | Only for stores of a constant null. A non-null store into a new (black) object keeps its barrier, for the insertion half. Go's compiler elides under the same condition | Yes | for review |
| D52 | 2026-10-12 | `Object.clone` copies raw words without barriers. What does `HybridBarrier` do? | Shade every reference field of the source before the copy (Go's `bulkBarrierPreWriteSrcOnly`): in `clone_in_heap` and in a flag-gated leaf call before C2's clone. Implemented with `memory_region_copy_pre` and an empty destination slice, so no mmtk-core API change | Yes | for review |
| D53 | 2026-10-12 | Arraycopy into an uninitialised destination (`IS_DEST_UNINITIALIZED`, e.g. C2 tightly-coupled copies) | Shade the source only; never read the destination, which can hold stale words | Yes | for review |
| D54 | 2026-10-12 | C1 stores whose field offset needs patching do not know the slot | Call `object_probable_write(src)` (shade all old fields, rescan `src` later) through the existing patching stub | Yes | for review |
| D55 | 2026-10-12 | `on_slowpath_allocation_exit` (`object_probable_write`) for `HybridBarrier` | No-op, as for SATB: after D51, no non-null store into a new object is unbarriered | Yes | for review |
| D56 | 2026-10-12 | The SATB weak-reference load barrier (x86 interpreter, C1, C2) is needed again for `HybridBarrier` with another flag | Move it into the shared `MMTkBarrierSet{Assembler,C1,C2}` with the flag address as a parameter; SATB passes `CONCURRENT_MARKING_ACTIVE`. SATB's C1 test changes from `== 1` to `!= 0` (same for a 0/1 flag) | Yes | for review |
| D57 | 2026-10-12 | How to test barrier paths before concurrent marking exists | Binding-only env var `MMTK_HYBRID_BARRIER_DEBUG_ALWAYS_ON=1` sets the flag once at start-up. mmtk-core drops the buffers while no marking is in progress | Yes | for review |
| D58 | 2026-10-12 | aarch64 `HybridBarrier` cannot be tested on this machine | Not implemented. Its selection is compiled only on x86_64 (`#ifdef AMD64`); on aarch64, `ConcurrentSpan` fails at start-up with the existing "Unimplemented" guarantee | Yes | open |

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

## D9. Where the pacer plugs in

**Question.** The design wires the pacer in as `GCTriggerSelector::FeedbackPacer`. On OpenJDK, mmtk-openjdk sets the GC trigger from `-Xms`/`-Xmx` (`FixedHeapSize` or `DynamicHeapSize`), so a new selector would need a binding change.

**Decision.** `ConcurrentSpan::collection_required` asks the pacer (`pacing.rs` around `pacer.rs`). The trigger's heap size is the hard limit, and also the memory limit (`soft_mem_limit` overrides it). Options `gc_percent` (-1 for off) and `soft_mem_limit`. `heapLive` is `SpanSpace`'s refill-charged counter plus other spaces' used pages; the trigger is checked at every span refill (Go's `mallocgc`), not only when pages are acquired.

**To undo.** Add the selector and move `Pacing` into a `GCTriggerPolicy`.

## D10. Allocation stalls

**Decision.** `GCTrigger::poll_or_stall` returns `Stall` when the plan's `stall_instead_of_pause` says so (marking in progress, not done). `Space::acquire` then calls `block_for_gc` and takes the pages afterwards; MMTk's existing block primitive returns at the end of the next pause, which is FinalMark (or a restart, after which the mutator may stall again). While a mutator stalls, all GC workers are active. Option `concurrent_span_allocation_stalls` (on); off gives `ConcurrentImmix`'s behaviour (FinalMark finishes marking stop-the-world).

**To undo.** Turn the option off, or remove the hook.

## D11. `System.gc()`

**Decision.** Stop-the-world `Pause::Full`. A Go-faithful version needs a race-free "wait until this cycle ends" primitive in the binding.

## D13. Ragged flush placement

**Decision.** The plan's `on_concurrent_work_drained` adds a `RaggedFlush` packet (with `add_no_notify`, because the caller holds the worker monitor lock) and returns `true`. The packet calls `Collection::flush_mutators_concurrently` (mmtk-openjdk: `Handshake::execute`) and compares a counter of non-empty barrier flushes before and after.

## D16. Collection attempts and concurrent cycles

**Decision.** `ScheduleCollection` skips `set_collection_kind` when the plan's concurrent work is in progress (the pause closes a cycle). This also affects `ConcurrentImmix`, whose FinalMark no longer counts as an attempt.

**To undo.** Revert the check in `ScheduleCollection` (and stalls then need another fix).

## D20. Default minimum heap

**Question.** Go's `defaultHeapMinimum` (`runtime/mgcpacer.go:59-60`) is 4 MiB, or 512 KiB when the `HeapMinimum512KiB` experiment is on. The `internal/goexperiment` sources are not in the reference checkout.

**Options.** 1. 4 MiB. 2. 512 KiB. 3. An option.

**Decision.** Option 1: the experiment is off by default in Go, and `TestGcPacer`'s `LargeHeapSlowAlloc` checks the runway against this constant.

**To undo.** Change `DEFAULT_HEAP_MINIMUM` in `src/plan/concurrent/span/pacer.rs`, or make it an option.

## D21. Pacer sharing and memory ordering

**Question.** Go's `gcControllerState` mixes atomics with fields only written with the world stopped. MMTk's `GCTriggerPolicy` is shared and its methods take `&self`.

**Options.** 1. Plain fields behind a `Mutex`. 2. Every field atomic, `&self` methods. 3. Go's split: atomics plus `&mut self` for STW methods.

**Decision.** Option 2, with `Relaxed` ordering. `revise` and `update` run on hot allocation paths during marking and must not take a lock; Go documents the races as benign. Methods that Go only calls with the world stopped (`start_cycle`, `end_cycle`, `reset_live`, `commit`) say so, and the caller provides the exclusion. Recorded as deviation PACER-7.

**To undo.** Wrap the pacer in a `Mutex` in the integration, or change `ORD` in `pacer.rs` to `SeqCst`.

## D22. Random streams in the pacer tests

**Question.** `TestGcPacer`'s `random(amp, seed)` streams use Go's `math/rand`, whose generator needs a 607-entry seed table that is not in the reference checkout and cannot be fetched.

**Options.** 1. Reconstruct Go's generator (needs the table, or about 7.8e12 generator steps to recompute it). 2. Use another deterministic generator with the same seeds and amplitudes. 3. Drop the three scenarios that use it.

**Decision.** Option 2 (SplitMix64). The scenarios check statistical bounds, not exact values, and all three pass Go's tolerances. Recorded as deviation TEST-1.

**To undo.** Port Go's `math/rand` `rngSource` into `pacer/tests.rs` once its source is available.

## D23. Branch for the pacer work

**Question.** The worktree for the pacer agent was created on a branch based on `master`, which lacks the design documents. Resetting that branch to `mmtk-core-go-gc` was refused as destructive.

**Decision.** Create a new branch `concurrent-span-pacer` from `mmtk-core-go-gc` in the same worktree and commit there. The original worktree branch had no commits of ours and is left as it was.

**To undo.** Cherry-pick the pacer commits onto any other branch.
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

## D50. The flag tested by compiled code

**Question.** The binding already has `CONCURRENT_MARKING_ACTIVE` (set in `resume_mutators` for SATB plans). `HybridBarrier` has its own flag in mmtk-core.

**Options.** 1. Test the binding's flag. 2. Test mmtk-core's flag through `barrier_active_address()`.

**Decision.** Option 2. The barrier's semantics (in mmtk-core) also test that flag, so the fast path and the slow path cannot disagree, and the flag changes exactly when the plan changes it (design §5.6). The C++ side reads the address once (`hybrid_barrier_active_address()`) and embeds it in generated code.

**To undo.** Replace `hybrid_barrier_active_address()` in `mmtkHybridBarrier.hpp` (mmtk-openjdk).

## D51. Barrier elision for new objects in C2

**Question.** `MMTkBarrierSetC2::can_remove_barrier` removes the barrier for any store into a just-allocated object. For SATB that is sound (the old value is null). For a hybrid barrier the new value must still be shaded: the new object is allocated black and is never scanned, and the stored value may only be reachable from a stack that is scanned later.

**Options.** 1. Never elide. 2. Elide only constant-null stores. 3. Elide, and shade the new object's fields on `on_slowpath_allocation_exit` (only covers slow-path allocations, so unsound).

**Decision.** Option 2. Go's `needwb` (`cmd/compile/internal/ssa/writebarrier.go`) elides the barrier on zeroed new objects only when the stored value is not a heap pointer, which is the same condition.

**To undo.** Change `MMTkHybridBarrierSetC2::object_reference_write_pre`.

## D52. `Object.clone`

**Question.** HotSpot clones by copying raw words (`clone_in_heap`, and C2's `clone` intrinsic through a basic `ArrayCopyNode`). SATB does nothing. The clone is a new black object, so the copied references need the insertion half.

**Options.** 1. `object_probable_write(dst)` after the copy (shades the copied values, but also schedules a pointless rescan, and there is no "after" hook in C2's intrinsic without more graph surgery). 2. Shade the source's reference fields before the copy, as Go's `bulkBarrierPreWriteSrcOnly` and Shenandoah's clone barrier do. 3. Add a clone API to mmtk-core.

**Decision.** Option 2, in the binding. Instance fields are shaded per oop map block (each a contiguous slot range), arrays as one range, both through `memory_region_copy_pre` with an empty destination slice (`mmtk_array_copy_pre_src_only`). `HybridBarrierSemantics::memory_region_copy_slow` iterates the two slices independently, so this shades exactly the source values. Object arrays cloned by C2 go through the arraycopy stubs, which are covered by D53. C1 has no clone intrinsic.

**To undo.** Remove `object_clone_pre` (mmtk-openjdk `mmtkBarrierSet.hpp`, `mmtkHybridBarrier.cpp`). If mmtk-core gains a source-only slice API, switch to it.

## D53. Arraycopy into an uninitialised destination

**Question.** C2 skips zeroing a new array that is filled by an arraycopy (`IS_DEST_UNINITIALIZED` stubs). `memory_region_copy_pre` would load the destination's old words, which may be stale pointers to freed cells.

**Decision.** For `IS_DEST_UNINITIALIZED` (stub prologue and runtime), call `mmtk_array_copy_pre_src_only`, which shades the source only. Other barriers keep their behaviour through a new virtual `object_reference_array_copy_pre_dst_uninitialized` that defaults to the old call.

**Note for review.** `SATBBarrier`'s prologue also calls `mmtk_array_copy_pre` for uninitialised destinations, and SATB's `memory_region_copy_slow` reads the destination. On a free-list space this could enqueue a stale word. Not changed here.

**To undo.** Revert the prologue branch in `mmtkHybridBarrierSetAssembler_x86.cpp` and the runtime override.

## D54. C1 stores that need field patching

**Question.** When C1 compiles a store to a field of an unloaded class, the offset is patched on first execution, so the slot address is not known when the barrier stub is generated. The field barrier's patching stub falls back to `object_probable_write(src)`.

**Decision.** Reuse that stub. For `HybridBarrier`, `object_probable_write` shades every old field of `src` (deletion half) and schedules `src` for a rescan when the buffers are flushed, after the store (insertion half). It is rare (only until the code is recompiled).

**To undo.** Implement exact patching in `MMTkC1HybridBarrierStub`.

## D55. `object_probable_write` after C2 slow-path allocation

**Decision.** A no-op for `HybridBarrier`, as for SATB. With D51, the only unbarriered stores into a new object store null.

**To undo.** Implement `MMTkHybridBarrierSetRuntime::object_probable_write`.

## D56. Sharing the weak-reference load barrier

**Question.** `HybridBarrier` needs the same weak-reference load barrier as SATB (interpreter `Reference.get`, C1, C2 including the unknown-referent check), gated by a different flag. Copying it would make a third copy (the field barrier has one).

**Decision.** Move SATB's code into `MMTkBarrierSetAssembler::weak_reference_load_barrier` (x86), `MMTkBarrierSetC1::weak_reference_load_at_resolved` and `MMTkBarrierSetC2::weak_reference_load_at_resolved`, with the flag address as a parameter. SATB and Hybrid call them. aarch64 SATB and the field barrier are untouched. `ConcurrentImmix` was re-run as a regression check.

**To undo.** Revert the refactoring part of the mmtk-openjdk commit.

## D57. Forcing the barrier on for testing

**Decision.** If `MMTK_HYBRID_BARRIER_DEBUG_ALWAYS_ON=1` and the plan uses `HybridBarrier`, the binding stores 1 to the flag after creating MMTk (`openjdk_gc_init`) and prints a line to stderr. Nothing clears it before concurrent marking exists. While no marking is in progress, `HybridBarrierSemantics` drops what it buffers, so this tests that every path reaches mmtk-core with valid arguments, not that marking is correct.

**To undo.** Remove `maybe_force_hybrid_barrier_on` in mmtk-openjdk `mmtk/src/api.rs`.

## D58. aarch64 (open)

**Question.** Design §9 asks for x86_64 and aarch64. aarch64 cannot be built or tested on this machine.

**Decision.** Not implemented. `mmtkHybridBarrier.cpp` and the barrier selection are compiled only under `AMD64`, so aarch64 builds still compile; selecting `ConcurrentSpan` there fails at start-up. Needs an aarch64 port of `mmtkHybridBarrierSetAssembler_x86.cpp` (store pre-barrier, arraycopy prologue, weak load, C1 stub) and a machine to test it.

**To undo.** n/a.

