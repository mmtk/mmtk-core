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
| D11 | 2026-10-12 | Is `System.gc()` a concurrent cycle that blocks the caller (design §5.2)? | Yes (changed later the same day, 82c520eff0): the caller waits for any cycle in progress, then for a whole new concurrent cycle, through `Collection::block_mutator_until`; option `concurrent_span_concurrent_user_gc`. Without blocking support it is a stop-the-world `Pause::Full` | Yes | for review |
| D12 | 2026-10-12 | How many times may FinalMark restart concurrent marking? | At most `concurrent_span_max_mark_restarts` (8); then FinalMark finishes the marking in the pause. Go has no limit | Yes | for review |
| D13 | 2026-10-12 | Where do ragged flush rounds run? | As a `RaggedFlush` packet in `Concurrent`, not in the last-parked-worker callback, which holds the worker monitor lock: a handshake there can deadlock with a mutator requesting a GC. Without `flush_mutators_concurrently`, FinalMark is requested directly | Yes | for review |
| D14 | 2026-10-12 | `ConcurrentPlan::on_concurrent_work_drained` must be able to add work or ask for a pause | It returns `bool`; `true` makes the last parked worker look again. Changes `ConcurrentImmix` and LXR (return `false`). Candidate upstream change, now inside a port commit | Yes | for review |
| D15 | 2026-10-12 | Soft references and finalizers make FinalMark trace (design §5.14) | Soft referents are strong in non-emergency cycles (`memory_manager::soft_references_are_strong`, used by mmtk-openjdk); finalizable objects' contents were traced while marking (`ScanFinalizableContents`), withdrawn by D88: `Finalization` traces in `FinalMark` | Yes | for review |
| D16 | 2026-10-12 | Does a pause that closes a concurrent cycle count as a collection attempt for out-of-memory? | No. FinalMark continues the attempt started by InitialMark; otherwise a stalled mutator saw a false emergency and OOM | Yes | for review |
| D17 | 2026-10-12 | How are assists parked (design §5.11 Stage A)? | New optional `Collection::block_mutator_until` / `wake_blocked_mutators`; mmtk-openjdk blocks the thread in the VM and waits on `gc_lock` (1 ms timed waits) | Yes | for review |
| D18 | 2026-10-12 | Weak→strong block during mark termination (design §5.14) | Not implemented. It is not needed for correctness: MMTk processes references inside the FinalMark pause, so no mutator can turn a weak reference strong while referents are cleared, and before that pause weak loads shade while the flag is set (until the end of FinalMark). The cost is liveness of termination: a weak→strong load after the work drains creates work, so FinalMark restarts (capped by `concurrent_span_max_mark_restarts`). The DaCapo matrix shows no runaway restarts | — | open |
| D19 | 2026-10-12 | Work split between the integrator and agents in this session | Pacer (D20–D23) and SpanSpace (D30–D38) by agents in worktrees, merged by the integrator; mmtk-openjdk `HybridBarrier` (D50–D58) by an agent in the shared checkout | — | for review |
| D20 | 2026-10-12 | Which `defaultHeapMinimum` (Go has 4 MiB, or 512 KiB under the `HeapMinimum512KiB` experiment)? | 4 MiB, Go's default | Yes | for review |
| D21 | 2026-10-12 | How is the pacer shared between mutators, workers and pauses? | All fields atomic (`Relaxed`), all methods `&self`; STW-only methods document their precondition | Yes | for review |
| D22 | 2026-10-12 | Go's `math/rand` source is not in the reference checkout. How are `TestGcPacer`'s random streams ported? | SplitMix64 with the same seeds and amplitudes | Yes | for review |
| D23 | 2026-10-12 | The pacer agent's worktree branch was created from `master`, not `mmtk-core-go-gc`, and a hard reset was refused | Work on a new branch `concurrent-span-pacer` created from `mmtk-core-go-gc` in the same worktree; the original worktree branch is left untouched | Yes | for review |
| D24 | 2026-10-12 | What feeds the pacer's memory-limit goal (Go: `mappedReady`, `heapFree`, `totalAlloc - totalFree`)? | `mappedReady` = all pages MMTk has reserved; `heapFree` = free pages in SpanSpace's page regions; heap object bytes = SpanSpace `alloc_bytes` (Go's `totalAlloc - totalFree`: charged at refill, corrected at uncache, reduced by sweeping) plus the other spaces' used pages. `heap_live` was tried first: right after marking it excludes the unswept garbage, which then looked like non-heap memory and pulled the goal down to the marked size. Counting whole spans was tried next: it hides span fragmentation, so the hard limit started the cycles again. Reported before every trigger check and at the end of a cycle. Before this nothing was reported, so the goal was the whole limit and the hard limit started every cycle | Yes | for review |
| D25 | 2026-10-12 | Sweeping before growing the heap and before starting a cycle | Both, as Go: `alloc_span` sweeps (until a page fits or a region goes back to MMTk) before acquiring a region (`mheap.reclaim`), and `collection_required` sweeps until the trigger no longer holds (`gcStart`). Unswept garbage otherwise counts against MMTk's hard limit | Yes | for review |
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
| D59 | 2026-10-12 | How are stores into VM roots (OopStorage, JNI global refs) barriered? | The MMTk `AccessBarrier` implements `oop_store_not_in_heap` and the not-in-heap cmpxchg/xchg; `HybridBarrier` shades the new value with `mmtk_shade_objects` when its flag is set. Only the new value | Yes | for review |
| D60 | 2026-10-12 | mmtk-openjdk's C++ `MMTkMutatorContext` did not mirror the new `SpanAllocator`, which crashed every plan | Mirror it, check `SPAN_ALLOCATOR_SIZE` and the whole `MUTATOR_SIZE` when binding a mutator, and take the allocation slow path for `TAG_SPAN` until an inline fast path exists | Yes | for review |
| D70 | 2026-10-12 | Where does Green Tea's per-span state (`scans`, `owned`, `marks`) live? | In the span descriptor, next to the existing mark bits; no inline block at the span page tail, and `nelems` unchanged (deviation GT-1) | Yes | for review |
| D71 | 2026-10-12 | How does the plan's concurrent marking (roots, barrier flushes, finalizable contents, `FinalMark` finishing) use `SpanTraceObjects` without rewiring every producer? | A `ConcurrentPlan::trace_objects_concurrently` hook, called at the start of `ConcurrentTraceObjects::do_work` (which `ProcessModBufSATB` also runs). `ConcurrentSpan` takes the objects when Green Tea is on. Other plans keep the default | Yes | for review |
| D72 | 2026-10-12 | What plays the part of Go's stealable span rings (`spanSPMC`)? | MMTk work packets: spilled spans become a new `SpanTraceObjects` packet; "chain empty" means the last spilled packet has started (GT-3) | Yes | for review |
| D73 | 2026-10-12 | A span scan knows object indices; how does it get object references? | Learn the reference offset within the cell from the first object marked for the span queue, and assert it is constant. Spans with padded cells use the object queue (GT-5) | Yes | for review |
| D74 | 2026-10-12 | Use Green Tea in stop-the-world `Full` pauses and reference processing too? | No: only in the concurrent marking packets. The generic `trace_object` sets scan bits, so both paths can meet in one span (GT-6) | Yes | for review |
| D75 | 2026-10-12 | Background sweeping that ends without a pause leaves the `Concurrent` bucket enabled, so the next `InitialMark`'s root packets ran inside the pause, before `Prepare` (found by the Green Tea graph test; marking then lost every object) | `on_pause_start` disables `Concurrent` at the start of every pause that starts a cycle, as it already did for `FinalMark`. The scheduler reopens it after the pause if it holds work | Yes | for review |

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

**Decision.** First a stop-the-world `Pause::Full`, because waiting for a concurrent cycle with `block_for_gc` is racy. Once `Collection::block_mutator_until` existed (for assists, D17), `System.gc()` became Go's `runtime.GC()`: wait for the cycle in progress, then request a cycle and block only the caller until `completed_cycles` advances.

**To undo.** `MMTK_CONCURRENT_SPAN_CONCURRENT_USER_GC=false`.

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

**Update.** Now that `ConcurrentSpan` marks concurrently, the plan clears the flag at the end of the first FinalMark, so the variable only forces the barrier on until the first cycle ends. Real marking now exercises the barrier anyway.

**To undo.** Remove `maybe_force_hybrid_barrier_on` in mmtk-openjdk `mmtk/src/api.rs`.

## D58. aarch64 (open)

**Question.** Design §9 asks for x86_64 and aarch64. aarch64 cannot be built or tested on this machine.

**Decision.** Not implemented. `mmtkHybridBarrier.cpp` and the barrier selection are compiled only under `AMD64`, so aarch64 builds still compile; selecting `ConcurrentSpan` there fails at start-up. Needs an aarch64 port of `mmtkHybridBarrierSetAssembler_x86.cpp` (store pre-barrier, arraycopy prologue, weak load, C1 stub) and a machine to test it.

**To undo.** n/a.


## D59. Stores into VM roots

**Question.** VM roots are scanned once, in the InitialMark pause. An object stored into one later (e.g. a new JNI global ref) from a stack that has not been scanned yet is lost if the stack then drops it (design §5.6).

**Options.** 1. Shade the new value in `oop_store_not_in_heap` and the not-in-heap atomics. 2. Rescan VM roots in FinalMark.

**Decision.** Option 1, as the design says. The `AccessBarrier` calls a new runtime hook `object_reference_write_not_in_heap(new_value)` (a no-op for other barriers); the `HybridBarrier` calls `mmtk_shade_objects` (`memory_manager::shade_objects`) with the value when the flag byte is set and the value is not null. The old value is not logged: the root was scanned at InitialMark, and any value stored since was shaded when it was stored. Native (JNI) code reaches roots only through the access API, so this covers JNI handles. Recorded as an extra relative to Go in design §11 (already listed).

**To undo.** Remove the not-in-heap accessors in mmtk-openjdk `mmtkBarrierSet.hpp`.

## D60. C++ mirror of the mutator

**Question.** mmtk-core added `span: [SpanAllocator; 2]` to `Allocators`. The binding's C++ mirror was not updated, so the barrier and config fields were read at wrong offsets and every plan crashed at start-up (`flush_remembered_sets` on GC worker threads). Then ConcurrentSpan hit `Unimplemented allocator fastpath` for the new allocator tag.

**Decision.** Mirror `SpanAllocator` (its `#[repr(C)]` layout) in `mmtkMutator.hpp`; export `SPAN_ALLOCATOR_SIZE` and `MUTATOR_SIZE` from Rust and check them in `MMTkMutatorContext::bind`, so a future change fails loudly; add `TAG_SPAN` (6) and force the allocation slow path for it in the x86 and aarch64 assemblers and C2, as for the free-list allocator. Allocation-heavy measurements of `ConcurrentSpan` therefore measure the slow path until the inline fast path (design §9, phase 2) exists.

**To undo.** Implement the SpanAllocator fast path; the size checks should stay.

## D70. Green Tea's per-span state

**Question.** Go keeps `spanInlineMarkBits` (`scans[63]`, `owned`, `marks[63]`, `class`) in the last 128 B of each one-page span, uses those marks instead of `gcmarkBits`, and ORs them into `gcmarkBits` at sweep. Design §5.12 planned the same, with `nelems` reduced as in `initSpan`.

**Options.** 1. As Go: an inline block at the page tail, a second mark bitmap, and a merge at sweep. 2. Keep the marks in the descriptor's `mark_bits` (SP1) and add the scan bits and `owned` to the descriptor.

**Decision.** Option 2 (deviation GT-1). The protocol is unchanged (`tryAcquire`, `release`, `marks &^ scans`), there is no second mark bitmap to merge, `is_live` and sweeping are unchanged, and the generated size-class tables stay valid. The cost is that marking touches the descriptor rather than the span page, which can change cache behaviour; this is recorded for the evaluation.

**To undo.** Move `scan_bits`/`owned` (and the marks for these classes) to the span page tail in `span_queue.rs`, reserve 128 B in `SpanClass::nelems` for `uses_span_queue` classes, and OR the marks into `mark_bits` in `Span::sweep_bits`.

## D71. Routing concurrent marking into `SpanTraceObjects`

**Question.** Roots (`ConcurrentMarkingRootsWorkFactory`), barrier buffers (`HybridBarrierSemantics`, `SATBBarrierSemantics`), rescans, `ScanFinalizableContents`, and `FinalMark`'s finishing of left-over packets all create `ConcurrentTraceObjects` or `ProcessModBufSATB` packets, in code shared with `ConcurrentImmix`. The integrator is editing `ConcurrentSpan`'s `global.rs` at the same time.

**Options.** 1. Plan-specific roots factory, barrier semantics and finalizer packet that create `SpanTraceObjects`. 2. One hook in `ConcurrentTraceObjects::do_work` that lets the plan trace the packet's objects with its own loop.

**Decision.** Option 2: `ConcurrentPlan::trace_objects_concurrently(worker, objects, already_marked) -> Option<Vec<ObjectReference>>`, defaulting to "give them back". It covers every producer, including packets moved to `FinishConcurrentWork` by `FinalMark`, and the plan-side change is one trait method. The cost is one dynamic call per packet. With the option off, the hook gives the objects back and the generic loop runs unchanged.

**To undo.** Remove the hook and its call; make the producers create `SpanTraceObjects` directly.

## D72. Stealable span queues

**Question.** Go's span queue spills into a chain of single-producer multi-consumer rings, which idle Ps steal from (`tryStealSpan`, guided by `work.spanqMask`). MMTk has no per-worker stealable queues of arbitrary items, but it has work packets, a per-worker local packet buffer that other workers steal from, and buckets.

**Decision.** Keep Go's ring (256) and spill rule (half when full; up to 16 every 64 puts, only if more than 4 and the previous spill has been taken). A spill is a new `SpanTraceObjects` packet carrying the spans (and holding no ownership change: the spans stay owned by the queue). "The previous spill has been taken" is an `Arc<AtomicBool>` set when that packet starts. A packet drains its own queues before it ends, so no span is ever left queued in a finished packet.

**To undo.** Replace the spill in `greentea.rs` with per-worker SPMC rings and a steal step in the worker loop (a scheduler change).

## D73. Object references from object indices

**Question.** `scan_span` scans objects by index, but `Scanning::scan_object` takes an `ObjectReference`, and MMTk has no API from object start to reference (MockVM's references are 8 bytes into the object; `SpanAllocator` can pad within a cell for an allocation offset).

**Options.** 1. A new `ObjectModel` method (API change). 2. VO bits (a feature that ConcurrentSpan does not require). 3. Learn the offset from marked objects and check it.

**Decision.** Option 3. `SpanSpace` stores the offset of the first object it marks for the span queue and asserts that every later one has the same offset. Allocations padded within their cell set the span's `padded` flag, and such spans use the object queue. Only objects allocated before marking can be found by a span scan (objects allocated during marking have their scan bit), so a span's flag is stable for those objects.

**To undo.** Add `ObjectModel::object_start_to_ref` (or similar) and use it in `SpanSpace::scan_span`.

## D74. Green Tea outside concurrent marking

**Decision.** `Full` pauses (only `System.gc()` and the debug option) and the tracing done by reference processing and finalization in `FinalMark` keep the generic trace. The generic `SpanSpace::trace_object` sets the scan bit of a small object it enqueues, so a span scan never scans it again, and the debug check at sweep (every marked object in a span-queue span has its scan bit) holds on every path.

**To undo.** Give the `Full` pause's work context a trace that uses `SpanTraceObjects`.

## D75. `Concurrent` bucket left enabled after background sweeping

**Question.** When background sweeping drains without a pause, `on_concurrent_work_drained` ends the concurrent phase, but the scheduler only closes `Concurrent` at the end of a pause. The next `InitialMark` added its root packets to an enabled, open bucket, so GC workers traced roots inside the pause before `Prepare`, against the previous cycle's mark bits; `prepare_initial_mark` then swept those marks away and the cycle marked nothing. Found by `mock_test_concurrent_span_greentea_graph` (second cycle), with Green Tea on or off. It is not a Green Tea change.

**Decision.** In `on_pause_start`, after dropping left-over sweep packets, disable `Concurrent` for every pause that starts a cycle (it was already disabled for `FinalMark`). The scheduler re-enables it when the pause ends if it holds work. Committed on its own, before the Green Tea commits, so the integrator can take it separately.

**To undo.** Remove the `set_enabled(false)` line in `on_pause_start`.

## D80. The allocate-black flag tested by the inline allocation fast path

**Question.** Go's malloc fast paths bail out while `gcBlackenEnabled` is set (design §5.8). Which flag should mmtk-openjdk's inline `SpanAllocator` fast path test? Candidates: the binding's `CONCURRENT_MARKING_ACTIVE` (set in `resume_mutators` from `concurrent_work_in_progress`), mmtk-core's `HYBRID_BARRIER_ACTIVE` (`barrier_active_address()`, not set in SATB mode), or the space's own `allocate_as_live`, which `SpanSpace::initialize_object_metadata` tests to allocate black.

**Decision.** The space's `allocate_as_live` flag, so that the fast path and the slow path cannot disagree (a mismatch would leave an object allocated during marking unmarked, and it would be freed while live). `policy` is private, so the binding cannot reach the flag itself: `AllocatorInfo::Span` gains `space_offset` (the allocator's `space` pointer, relative to the mutator) and `allocate_black_offset` (the flag, relative to the space). The fast path loads the space pointer from the mutator and tests the byte: one extra dependent, L1-resident load compared with an embedded global address, but no MMTk instance or mutator is needed at code-generation time and other bindings can use it the same way. The flag only changes while mutators are stopped (`ConcurrentSpan::set_marking_state`), and generated code has no safepoint between the test and the allocation.

**To undo.** Remove the two fields from `AllocatorInfo::Span` (mmtk-core) and test another flag in `span_allocate` / `expand_span_allocate` (mmtk-openjdk).

## D81. Size to span class in compiled code

**Question.** Go computes the size class with `size_to_class8` / `size_to_class128` and a branch. The binding needs the span class for constant sizes (most instances in C1/C2) and variable sizes (arrays, the interpreter's `_new`).

**Decision.** mmtk-core exports `span_allocator::span_class_index(size, align, noscan)`, the lookup that `SpanAllocator::alloc` uses. mmtk-openjdk builds a flat 4097-byte table, `size_to_span_class[size >> 3]` for sizes up to 32 KiB with an alignment of 8 and scan spans (OpenJDK allocates with an alignment of one word, no offset, from `Span(0)`). Constant sizes are looked up at compile time and become an immediate offset into the mutator's span array; variable sizes cost one byte load. Sizes with no small class map to class index 0, whose slot always holds the empty span, so the fast path fails safely on it. This is FP-1.

**To undo.** Emit Go's two-table lookup instead (the tables would need exporting).

## D82. Plain increment of `alloc_count` in compiled code

**Question.** `Span::next_free_fast` updates `alloc_count` with `fetch_add(Relaxed)` (a locked instruction on x86). Can compiled code use a plain load/add/store?

**Decision.** Yes. A cached span (`sweepgen` `h+1` or `h+3`) is written only by its owning mutator: sweepers only claim spans at `h-2` by CAS (`sweep_span`), and `uncache_span`/`flush` (which may sweep a stale span) run on the owner or in a pause with the owner stopped. `cache_span` and `refill` read `alloc_count` before the span is cached or on the owner. Other threads only read it in pauses. So the counter has a single writer while it is cached, which is also what Go assumes (`s.allocCount++` in `nextFreeFast`). The same holds for `alloc_cache` and `freeindex`, which the Rust code already writes with plain (relaxed) stores.

**To undo.** Use `lock add` (x86) / a `GetAndAddL` node (C2).

## D83. Zeroing on the inline fast path

**Question.** Go's `mallocgc` zeroes the object if the span's `needzero` is set. Should the binding's fast path test `needzero`?

**Decision.** No, except with `-XX:+ZeroTLAB`. MMTk runs OpenJDK with `UseTLAB` off, so C1 (`initialize_object`) and C2 (`InitializeNode`) always zero the object body after an inline allocation, and the interpreter's `_new` zeroes it unless `ZeroTLAB` is set. The object's bytes are therefore zeroed whatever `needzero` says. With `ZeroTLAB` the interpreter and C2 fast paths take the slow path for `needzero` spans (fresh spans are zero). The cell's tail beyond the object size is not zeroed by the fast path (Go zeroes the whole cell); nothing reads it. On fresh (already zero) spans OpenJDK zeroes the object again, which Go would not. This is FP-3.

**To undo.** Test `needzero_offset` unconditionally in `span_allocate` / `expand_span_allocate`.

## D84. Switching the fast path off

**Decision.** `MMTK_ENABLE_SPAN_ALLOCATION_FASTPATH=false` (mmtk-openjdk) makes every tier call the slow path for `SpanAllocator` allocations, as before this work. The existing `MMTK_ENABLE_ALLOCATION_FASTPATH=false` also does, but it also turns off C2's barrier elision for stores into new objects (`can_remove_barrier`), so it is not a like-for-like comparison. The fast path is also off when C2 has no `CountTrailingZerosL` rule (C2 only). Without BMI2, the x86 variable shift saves and restores `rcx` around the shift (`push`/`pop`) unless the temporary already is `rcx`.

**To undo.** Remove the env var from `MMTkHeap::initialize`.

## D85. Post-allocation work on the inline fast path

**Question.** What does `post_alloc` do for `SpanSpace`, and must the fast path do it?

**Decision.** `SpanSpace::initialize_object_metadata` sets the VO bit (only with the `vo_bit` feature) and, if `allocate_as_live`, allocates black. The fast path is never taken while allocating black (D80). With `MMTK_VO_BIT`, the span fast path is followed by the same inline VO-bit code as the bump-pointer fast path (interpreter, C1, C2); that combination has not been tested. Nothing else is needed: `SpanAllocator::alloc` does no other work after `next_free_fast` for OpenJDK (no padding: alignment 8, no offset).

## D86. aarch64 and the MMTk C1 runtime stubs

**Decision.** Only x86_64 has the inline fast path (interpreter, C1, C2 on x86_64). aarch64's `eden_allocate` still takes the slow path for `TAG_SPAN`. C2's expansion is architecture-neutral IR, but it is enabled only under `#ifdef AMD64` until it has been tested elsewhere.

**To undo / finish.** Port `span_allocate` to `mmtkBarrierSetAssembler_aarch64.cpp`.

## D87. Core API committed inside another commit

**What happened.** The mmtk-core part of the fast path (`AllocatorInfo::Span::{space_offset, allocate_black_offset}`, `span_allocator::span_class_index`, `SpanSpace::allocate_as_live_offset`, `SpanAllocator::space_offset`, and `mock_test_concurrent_span_inline_fastpath`) was left uncommitted in the shared worktree for a few minutes and was picked up by the integrator's commit `67e2dd69f0` ("ConcurrentSpan: wire Go's GC CPU limiter into assists"). The commit was not rewritten (engineering §4). The follow-up `a0260aca83` ("Make the SpanAllocator inline fast-path mock test robust to GCs") records this and finishes the test.

**To undo.** If the branch is split for upstreaming, move these hunks out of `67e2dd69f0` into their own commit.

## D88. Finalizable objects are not scanned during marking

**What happened.** fop's marked bytes grew by 4-5 MB per DaCapo iteration under concurrent marking (16 → 52 MB over 10 iterations at 200 MB; at 39 MB the heap filled and the collector ran back to back), but stayed at 11-12 MB with stop-the-world marking or with `MMTK_NO_FINALIZER=true`. `ScanFinalizableContents` (Go's `gcScanFinalizer`, D15) scanned the contents of every finalizable candidate during marking. A finalizable object that is reachable from its own fields (any parent/child cycle) was then marked, so `Finalization` saw it as live: it was never finalized and never freed, with everything it reaches. Go documents this behaviour for finalizer cycles; Java requires such objects to be finalized.

**Decision.** Drop `ScanFinalizableContents`. `Finalization` in `FinalMark` resurrects the unreachable candidates and traces from them in the pause, as in `ConcurrentImmix`. The pause traces what finalizable garbage reaches, which is usually small. A correct concurrent version needs a separate "finalizably reachable" mark (as ZGC has), which is left for later.

