# Porting Go's garbage collector to mmtk-core

| | |
|---|---|
| Status | Draft design |
| Date | 2026-10-12 |
| mmtk-core base | `94b1ab83a2` (master, includes ConcurrentImmix, LXR) |
| Target binding | mmtk-openjdk `jdk-21` (`db71e7a`), OpenJDK `jdk-21.0.9` |
| Go reference | `golang/go` master `2c3c1caad2` (2026-10-10). Green Tea GC is on by default |
| Proposed plan name | `ConcurrentSpan` (`PlanSelector::ConcurrentSpan`) |
| Engineering | [Practices](concurrent-span-engineering.md), [upstreamable changes](concurrent-span-upstream-changes.md), [deviations from Go's code](concurrent-span-go-deviations.md), [decisions for review](concurrent-span-decisions.md) |

## 1. TL;DR

Go's collector is a concurrent, non-moving, non-generational, precise mark-sweep collector. It has two short stop-the-world (STW) pauses and a hybrid Yuasa/Dijkstra write barrier, and it allocates black during marking. It sweeps lazily and in the background, paces itself with a GOGC/GOMEMLIMIT feedback controller, and makes allocating goroutines pay for marking through *mark assists*. Its heap is a size-segregated span heap with 67 size classes.

The goal is to **evaluate Go's collector alongside other MMTk GCs and OpenJDK's own GCs**, on the same JVM and the same workloads. To do that, we implement it as a plan in mmtk-core and run Java on **OpenJDK through mmtk-openjdk**. Go is the reference algorithm, not a target runtime.

Because the point is evaluation, fidelity to Go's design matters: a result should say something about Go's collector, not about our port. Where a Go mechanism exists only because of how the Go runtime or compiler works, and OpenJDK has an equivalent, we use the OpenJDK equivalent, list the difference in §11, and say how it could affect the comparison.

mmtk-core already has most of the skeleton. `ConcurrentImmix` gives an InitialMark/concurrent/FinalMark pause machine, the `Concurrent` work bucket, and barrier plumbing. mmtk-openjdk already implements several MMTk barriers, including SATB, in the interpreter, C1 and C2. Native `MarkSweepSpace` gives a size-segregated free-list heap with lazy sweeping.

The port is therefore:

1. A new non-moving policy, `SpanSpace`, that reproduces Go's span heap: size classes, spans, a page→span map, per-mutator span caches, mark/alloc bitmap swapping, and `sweepgen`.
2. A new plan, `ConcurrentSpan`, built on the existing concurrent-plan machinery.
3. Its own barrier, `HybridBarrier`: Go's flag-gated barrier that logs both the old and the new value of every pointer write while marking. It is new in mmtk-core and in mmtk-openjdk's interpreter, C1 and C2.
4. A Go-style pacer as a `GCTriggerPolicy`.
5. Three scheduler/API additions that mmtk-core does not have today: concurrent sweeping outside a GC, concurrent per-thread stack scanning, and mutator assists.
6. On the OpenJDK side, Go-style stack scanning: each thread's stack is scanned once during marking, in a per-thread handshake, with no global pause. `HybridBarrier` is what makes that sound, as in Go.

Like Go, `ConcurrentSpan` has **no stop-the-world full GC**. The only pauses are the short InitialMark and FinalMark, and FinalMark is Go's mark termination: it does no tracing, and if it finds work it resumes concurrent marking instead (§5.2). When the heap runs out, an allocating thread stalls until the current concurrent cycle has freed memory, and every GC worker is put to work in the meantime (§5.11).

For comparison runs, `ConcurrentSpan` can also run with the existing SATB barrier, and with HotSpot's `StackWatermark` stack processing (the mechanism ZGC and Shenandoah use).

Green Tea span scanning and the scavenger come last.

We build it in phases. Each phase leaves a working plan that is tested on MockVM and on OpenJDK.

## 2. Scope

### What "port" means here

This document is about implementing **Go's GC algorithm and heap organisation inside mmtk-core**, as a plan and policy, and running it on OpenJDK so that it can be compared with:
- other MMTk plans on mmtk-openjdk, especially `ConcurrentImmix`, `LXR`, `Immix`, `GenImmix` and `MarkSweep`;
- OpenJDK's own collectors: G1, Parallel, ZGC and Shenandoah.

The core stays VM-neutral, as mmtk-core requires: every OpenJDK-specific concern is behind the `VMBinding` interface, and MockVM tests cover the core.

Running Go programs on MMTk is not a goal. There is no Go binding track.

### Goals

1. A `ConcurrentSpan` plan whose GC cycle, pause structure, allocate-black rule, sweeping, pacing and assists match Go's (§3), with deviations listed explicitly, along with their likely effect on the evaluation (§11).
2. A `SpanSpace` policy with Go's size classes and span layout.
3. A pacer whose trigger and goal arithmetic reproduce `mgcpacer.go`, verified against Go's own pacer simulation tests.
4. `ConcurrentSpan` runs DaCapo on mmtk-openjdk on x86_64 and aarch64, the two ports that have the SATB barrier today.
5. Results that are comparable with other collectors: the same harness, heap configurations and metrics, and switches for the main Go features so their effect can be measured on its own (§10.1).
6. All new mmtk-core mechanisms (concurrent sweep, concurrent stack scan, assists) designed as reusable infrastructure, not `ConcurrentSpan`-only hacks. `ConcurrentImmix` should be able to use the concurrent stack scan and the pacer too, which also helps separate the effect of the heap from the effect of the pacer.

### Non-goals

- A Go binding, and any change to the Go runtime or compiler.
- Go-only heap features that Java does not need:
  - interior-pointer resolution as a public API (Java references always point to the object start);
  - the tiny allocator, the malloc header and span-tail pointer bitmaps;
  - conservative scanning of async-preempted frames;
  - Go's specials (finalizer, cleanup, weak handle) and their sweep-time semantics. Weak references and finalizers use MMTk's own mechanism instead (§5.14).
- Go user arenas, `RuntimeFreegc`, `runtime.Pinner`, goroutine-leak detection and cgo pointer checks.
- Generational or moving variants.
- The scavenger, which returns memory to the OS, beyond a stub interface. It is a late milestone.

### Naming and attribution

**No "Go" in identifiers.** The plan is called `ConcurrentSpan`, after the space it is built on (`SpanSpace`), the same way `ConcurrentImmix` is named after `ImmixSpace`. No identifier, option or plan name contains "Go":
- "Go" is a Google trademark. Go's brand guidelines forbid combining it with a product or feature name in a way that implies official status. A name like "GoGC" reads as "the Go GC".
- A plan called "GoGC" would also clash with Go's `GOGC` setting.
- The plan deliberately differs from Go in places (§11). A Go-branded name would invite "Go doesn't do that" reports whenever it drifts from upstream.

Prose in docs and module comments may say "a port of Go's garbage collector". That is a plain factual reference and needs no permission.

**Attribution for ported code.** Go is licensed under BSD-3-Clause. Every file that is translated from Go source or generated from Go's generators carries Go's copyright notice and licence text in its header. Today that means the pacer, the CPU limiter and the generated size-class tables. Those files are also listed in mmtk-core's `COPYRIGHT` as an exception to the MIT/Apache-2.0 default. BSD-3-Clause is compatible with both.

Clause 3 of that licence forbids using the names of Google or the Go contributors to endorse or promote derived work. So the attribution says the code is *derived from* Go and implies no endorsement.

Go's separate patent grant covers Google's own Go implementation. Whether it extends to a reimplementation should be checked with counsel before release.

## 3. Go's collector, as the reference

All Go paths are relative to `go/src/`. This section describes Go as it is. §5 says which parts `ConcurrentSpan` adopts on OpenJDK, and §11 lists what it does differently.

### 3.1 Cycle

`runtime/mgc.go` uses three phases: `_GCoff`, `_GCmark` and `_GCmarktermination` (`mgc.go:250`). `setGCPhase` turns the global `writeBarrier.enabled` flag on for the last two (`mgc.go:257`).

| Step | What Go does | Where |
|---|---|---|
| Trigger | `heapLive >= trigger`, checked after an mcache refill. Also a 2-minute forced GC and `runtime.GC()` | `mgc.go:706-725` |
| Pre-STW | The triggering goroutine sweeps any leftover spans, then starts per-P mark worker goroutines | `mgc.go:767`, `:817` |
| **STW 1: sweep termination** | `finishsweep_m`, start pacer cycle, enable write barrier, prepare root jobs, grey each P's tiny block, `gcBlackenEnabled=1`. **No roots are scanned in the pause** | `mgc.go:835-930` |
| Concurrent mark | Background workers, idle workers and assists drain root jobs (globals, each goroutine stack, specials) and the heap | `mgcmark.go` |
| Termination detection | Ragged barrier: every P flushes its write-barrier buffer and gcWork. Repeat until no P had work | `mgc.go:997-1156` |
| **STW 2: mark termination** | Flush write-barrier buffers again. If anything was found, restart the world and resume concurrent marking. Otherwise do no tracing, disable the barrier, `sweepgen += 2`, commit the pacer | `mgc.go:1066-1531` |
| Sweep | Background sweeper goroutine, plus proportional sweeping by allocators. Mark bitmaps become alloc bitmaps | `mgcsweep.go` |

### 3.2 Barrier

The barrier is described in `mbarrier.go:24-131` and proposal 17503:

```
writePointer(slot, ptr):
    shade(*slot)                 // Yuasa deletion
    if current stack is grey:
        shade(ptr)               // Dijkstra insertion
    *slot = ptr
```

- The stack-greyness condition is **not implemented**. Both values are always buffered (`mwbbuf.go:221-227`).
- Stack writes are not barriered. Globals are.
- The compiler emits `if writeBarrier.enabled { buf := gcWriteBarrierN(); buf[0]=new; buf[1]=*slot }` (`cmd/compile/internal/ssacompile/writebarrier.go:143-152`).
- The buffer is a 512-entry per-P `wbBuf`, flushed by `wbBufFlush1`.
- Bulk copies and clears go through `bulkBarrierPreWrite` (`mbitmap.go:388-466`), which shades destination old values and source values.

This barrier is what lets Go scan **each goroutine stack once, concurrently, with no rescan**. Once a stack is scanned it is black. Goroutines created during mark start black.

The insertion half is needed because Go scans a stack at some time *after* mark start, not as it was at mark start. `ConcurrentSpan` ports this barrier as `HybridBarrier` (§5.6) and scans stacks the same way (§5.7).

### 3.3 Mark work

- Each P has a `gcWork` of 2 KiB workbufs (`mgcwork.go`).
- Background workers run at 25% of GOMAXPROCS. Dedicated workers cover `round(0.25·P)`, and fractional workers cover the remainder (`mgcpacer.go:403-419`). Idle Ps also run idle workers.
- **Assists** work as follows (`mgcmark.go:499-824`):
  - Every allocation debits `g.gcAssistBytes`.
  - A goroutine in debt first steals background credit, then runs `gcDrainN` for `debt × assistWorkPerByte` scan work, with a 64 KiB minimum.
  - If it is still in debt, it parks until background workers pay it off.
- Allocate-black: `gcmarknewobject` marks each new object and adds it to `bytesMarked` (`mgcmark.go:1762`).
- Objects over 128 KiB are split into 128 KiB oblets.
- Specials:
  - Finalizer specials scan the object's *contents* as a root, without marking the object.
  - Weak-handle specials keep the handle alive but not the target.
  - Cleanup specials scan their fn/arg (`mgcmark.go:393-493`).
- Every pointer is resolved with `findObject`: page→span map, then a magic-number divide (`mbitmap.go:1363`). Interior pointers are normal.

### 3.4 Green Tea (default on)

Green Tea applies to one-page spans of 16–512 B objects (`mgcmark_greenteagc.go:258`). Marking an object sets an inline mark bit at the end of the span page. The marker then tries to take ownership of the span and queues the **span**, not the object, on a per-P span queue that other Ps can steal from.

`scanSpan` scans `marks &^ scans` in one pass, using the span-tail pointer bitmap. It uses AVX-512 when the span is dense. At sweep, inline marks are ORed into `gcmarkBits`.

### 3.5 Heap

| Item | Go | Reference |
|---|---|---|
| Size classes | 67 classes (class 0 = large) up to 32 KiB, on 8 KiB pages. Spans are 1–10 pages. Worst-case waste is 26.6% | `internal/runtime/gc/sizeclasses.go:84-102` |
| Span class | `sizeclass<<1 \| noscan` gives 136 span classes. Pointer-free objects live in separate spans that the GC never scans | `mheap.go:573-592` |
| Span descriptor | `mspan` holds `freeindex`, `allocCache`, `allocBits`, `gcmarkBits`, `sweepgen`, `divMul`, `specials` and `largeType` | `mheap.go:422-516` |
| Page→span | `heapArena.spans[8192]` per 64 MiB arena, plus per-page bitmaps `pageInUse`, `pageMarks`, `pageSpecials` | `mheap.go:268-338` |
| Pages | A radix-tree page allocator over 4 MiB chunks, address-ordered first fit | `mpagealloc.go` |
| Central | One `mcentral` per span class. Each holds `partial`/`full` × `swept`/`unswept` lock-free span sets | `mcentral.go` |
| Per-P cache | `mcache.alloc[136]` plus a tiny allocator for noscan objects under 16 B | `mcache.go` |
| Pointer metadata | Objects up to 512 B: bitmap at the span tail. 512 B–32 KiB: an 8-byte type header (the user pointer is slot+8). Large: `span.largeType` | `mbitmap.go:15-54` |
| Mark bits | Allocated per span per cycle. At sweep `allocBits = gcmarkBits` and fresh zeroed `gcmarkBits` are allocated, so marks are **never cleared** | `mgcsweep.go:704` |
| `sweepgen` | Relative to heap `h`: `h-2` needs sweep, `h-1` being swept, `h` swept, `h+1`/`h+3` cached in an mcache | `mheap.go:494-500` |

### 3.6 Pacer

The pacer is in `mgcpacer.go` (design 44167):

```
goal_gogc   = heapMarked + (heapMarked + lastStackScan + globalsScan) * GOGC/100
goal_limit  = memlimit - nonHeapOverhead - headroom
goal        = min(goal_gogc, goal_limit)
runway      = consMark * (1-u)/u * (lastHeapScan + lastStackScan + globalsScan),  u = 0.25
trigger     = clamp(goal - runway, heapMarked + 0.7*(goal-heapMarked), heapMarked + 0.95*(goal-heapMarked))
assistWorkPerByte = (expectedScanWork - workDone) / (goal - heapLive)
```

`goal_gogc` is floored at 4 MiB × GOGC/100. The memory-limit goal subtracts a headroom of max(3%, 1 MiB) and is floored at `heapMarked`.

A CPU limiter (`mgclimit.go`) caps GC CPU at about 50% by suppressing assists.

### 3.7 Weak semantics

- **Finalizers** resurrect the object at sweep, so it survives one extra cycle.
- **`AddCleanup`** never resurrects.
- **Weak handles** are zeroed at sweep, *before* any finalizer resurrection.
- **`weak.Pointer.Value()` during mark** shades the target, which acts like a deletion barrier. It blocks during the termination window.

`ConcurrentSpan` does not adopt these semantics. Weak references and finalizers use MMTk's reference and finalizer processing (§5.14).

## 4. What mmtk-core and mmtk-openjdk already give us

| Need | Existing piece | Fit |
|---|---|---|
| Pause machine | `Pause::{Full, InitialMark, FinalMark}` (`src/plan/concurrent/mod.rs:14`). `ConcurrentImmix` pause selection, `on_pause_start` and `on_pause_end` (`src/plan/concurrent/immix/global.rs:134-350`) | Good. Reuse the structure |
| Concurrent marking | `WorkBucketStage::Concurrent`, `ConcurrentTraceObjects` (`src/plan/concurrent/concurrent_marking_work.rs:50-109`). Leftover work moves to `FinishConcurrentWork` at FinalMark | Good |
| Drain → FinalMark | `poll_from_last_parked_worker` (`src/util/heap/gc_trigger.rs:175`) and `on_concurrent_work_drained` | Good |
| Concurrent work with no pause | `Collection::concurrent_work_finished_no_pause` (`src/vm/collection.rs:159`), used by LXR | Use for concurrent sweep |
| Worker count control | `WorkerMonitor::set_active_workers` caps how many workers run (`src/scheduler/worker_monitor.rs:137`). The scheduler sets it to `concurrent_threads` when concurrent work starts (`src/scheduler/scheduler.rs:728`) | Basis for assists (§5.11) |
| SATB barrier (core) | Object-remembering, unlog-bit filtered (`src/plan/barriers.rs:358-442`, `src/plan/concurrent/barrier.rs`) | Good |
| Barriers (OpenJDK) | A barrier framework with `MMTkSATBBarrier`, `MMTkObjectBarrier`, `MMTkFieldBarrier` and `MMTkUnlogBitBarrier` (`openjdk/share/barriers/`), each with interpreter, C1 and C2 support. SATB exists on x86_64 and aarch64 (`openjdk/cpu/{x86,aarch64}/mmtkSATBBarrierSetAssembler_*`) and logs `Reference.referent` loads, as in G1. No riscv SATB | `HybridBarrier` is a new member of this framework, modelled on SATB |
| Per-thread operations in HotSpot (JDK 21) | Handshakes (`runtime/handshake.hpp`): `Handshake::execute(cl)` runs a closure on every Java thread without a safepoint and can be called from a GC thread, as ZGC does. The single-thread synchronous form needs a Java thread as the caller. Also concurrent stack processing via `StackWatermark` (`runtime/stackWatermarkSet.hpp`), used by ZGC and Shenandoah. mmtk-openjdk uses none of these yet. Its GC workers are `NamedThread`s (`MMTkCollectorThread`) | Basis for concurrent stack scanning (§5.7) |
| Code roots (OpenJDK) | mmtk-openjdk keeps its own code-cache root set (`MMTkHeap::register_nmethod`, `ScanCodeCacheRoots` in `mmtk/src/gc_work.rs:52`), scanned with the VM roots. Stack walks only do on-stack bookkeeping (`MMTkBookKeepingCodeBlobClosure`) | Good. Stays in the InitialMark pause. nmethods registered during marking need the barrier (§5.6) |
| Allocate-black | `Space::set_allocate_as_live` (`src/policy/space.rs:424`). Honoured by Immix and LOS only | Must be added to the new policy |
| Size-segregated heap | Native `MarkSweepSpace` + `FreeListAllocator`: mimalloc bins, 64 KiB blocks, lazy or eager sweep | Partial. See §5.3 |
| Weak references and finalizers | mmtk-core's `ReferenceProcessors` and `FinalizableProcessor`. mmtk-openjdk registers `java.lang.ref.Reference` objects with `add_{soft,weak,phantom}_candidate` during scanning, and finalizable objects with `add_finalizer` | Good. Schedule the same packets as `ConcurrentImmix` |
| Heap trigger | `GCTriggerPolicy` with `FixedHeapSize`, `MemBalancer` and `Delegated` (`src/util/heap/gc_trigger.rs:382+`) | Add a `FeedbackPacer` policy |
| Tests | MockVM can run real GCs since #1408 and supports per-test VM configs (`define_mock_vm!`). CI runs `MMTK_PLAN=all` mock tests | Roots cannot be mocked yet |

**Gaps that block a faithful port:**

1. **Lazy sweeping is unsafe during concurrent marking.**
   - `MarkSweepSpace` clears side mark bits for every occupied chunk in `PrepareChunkMap` (`src/policy/marksweepspace/native_ms/global.rs:576-605`).
   - `FreeListAllocator::prepare` is empty (`src/util/alloc/free_list_allocator.rs:415`), so lazily-unswept blocks survive into the next GC.
   - If marking were concurrent, a mutator calling `recycle_local_blocks` would sweep such a block against *partial* marks and free live objects.
   - This is why `ConcurrentImmix` exists but there is no concurrent mark-sweep.
2. **No allocate-black in `MarkSweepSpace`.** `initialize_object_metadata` only sets the VO bit (`native_ms/global.rs:194`). Block states would also have to be marked, or `attempt_release` can free a block that holds only new objects.
3. **Stack scanning is STW-only.** In mmtk-core, `ScanMutatorRoots` is only created inside `StopMutators` (`src/scheduler/gc_work.rs:263-298`). In mmtk-openjdk, `scan_roots_in_mutator_thread` walks a stopped thread with `thread->oops_do` (`openjdk/share/mmtkUpcalls.cpp`).
4. **No hybrid barrier.** Every existing barrier fast path is per object, filtered by an unlog bit, which cannot express Go's insertion half. mmtk-openjdk has no barrier that logs new values.
5. **No mutator assists.** The only back-pressure is blocking in `Space::acquire`.
6. **No byte-accurate pacing.** Polls happen only when a space acquires fresh pages (`src/policy/space.rs:85`), and concurrent tracing does not count live bytes.
7. **No GC activity outside a GC.** Workers park when `GcStatus::NotInGC`, so there is no background sweeper.
8. **No scavenger, soft memory limit or CPU limiter.**
9. **OpenJDK's inline allocation fast path is bump-pointer only.** Compiled code reads the TLAB-style cursor and limit (`get_tlab_top_and_end_offsets`, `openjdk/cpu/x86/mmtkBarrierSetAssembler_x86.cpp:81`). A free-cell allocator gets no inline fast path, so every allocation calls into MMTk.

A side finding: `CommonPlan::release_nonmoving_space` calls `self.nonmoving.prepare(...)` instead of `release(...)` under `marksweep_as_nonmoving` (`src/plan/global.rs:914-915`). It is unrelated to this port but worth a separate fix.

## 5. Design

### 5.1 Module layout

```
src/plan/concurrent/span/
    mod.rs
    global.rs          ConcurrentSpan: Plan + ConcurrentPlan; pause selection; prepare/release
    mutator.rs         MutatorConfig: SpanAllocator for scan + noscan, HybridBarrier or SATBBarrier
    gc_work.rs         ConcurrentSpanGCWorkContext, root factories, SpanTraceObjects (span-aware)
    pacer.rs           FeedbackPacer: pure arithmetic, unit-tested against Go's TestGcPacer
    assist.rs          per-mutator assist debt, global bg credit, park + extra worker
src/plan/barriers.rs   + HybridBarrier, BarrierSelector::HybridBarrier (reusable by other plans)
src/scheduler/
    + ConcurrentScanStacks packet (concurrent stack scanning, reusable by any concurrent plan)
src/policy/spanspace/
    mod.rs
    sizeclasses.rs     generated from Go's mksizeclasses output
    span.rs            Span descriptor, sweepgen, bitmaps, divMagic
    pagemap.rs         page -> span id side table
    pages.rs           page allocator over 4 MiB chunks
    central.rs         per-class partial/full x swept/unswept sets
    sweep.rs           span sweep, concurrent sweep packets
    global.rs          SpanSpace: SFT, PolicyTraceObject, Space
src/util/alloc/span_allocator.rs   the mcache: per-class current span, allocCache fast path
src/util/heap/gc_trigger.rs        + GCTriggerSelector::FeedbackPacer (delegates to plan/concurrent/span/pacer.rs)
```

Registration follows the usual pattern:
- `PlanSelector::ConcurrentSpan` in `src/util/options.rs:29`;
- `pub mod span;` next to `pub mod immix;` in `src/plan/concurrent/mod.rs`;
- `create_plan` and `create_mutator` arms in `src/plan/global.rs`;
- the exhaustive match in `src/vm/tests/mock_tests/mock_test_allocator_info.rs`.

The OpenJDK-side changes are listed in §9.

### 5.2 Cycle and pauses

```
            ┌──────────────── GC off ────────────────┐
            │ mutators allocate; lazy + bg sweep      │
            │ pacer: heapLive >= trigger ?            │
            └───────────────┬────────────────────────┘
                            │ GCTrigger::poll from span refill (non-blocking)
                    ┌───────▼────────┐
                    │ InitialMark    │ STW: finish sweep, flip mark epoch,
                    │ (sweep term.)  │ enable barrier + allocate-black, scan VM roots
                    └───────┬────────┘ (stacks are not scanned yet)
                            │
          ┌─────────────────▼──────────────────────┐
          │ Concurrent mark                         │ workers (concurrent_threads),
          │  per-thread stack scans (handshake),    │ assist workers, barrier buffers
          │  heap tracing, ragged flush rounds      │ -> Concurrent bucket
          └─────────────────┬──────────────────────┘
                            │ a ragged flush round found no work
                    ┌───────▼────────┐
                    │ FinalMark      │ STW, no tracing: check barrier buffers.
                    │ (mark term.)   │ Work found: resume concurrent mark (back up).
                    └───────┬────────┘ Else: reference processing (no tracing),
                            │          pacer commit, disable barrier, sweepgen += 2
                            │
          ┌─────────────────▼──────────────────────┐
          │ Concurrent sweep (InConcurrentGC)       │ ends with
          │ + mutator lazy sweep                    │ concurrent_work_finished_no_pause
          └────────────────────────────────────────┘
```

**No STW full GC.** Go has no full-heap stop-the-world collection, and neither does `ConcurrentSpan`:
- `schedule_collection` only ever selects `InitialMark` (no cycle running) or `FinalMark` (marking has finished concurrently).
- `FinalMark` is only requested when the `Concurrent` bucket drains, never because an allocation failed. This differs from `ConcurrentImmix`, which answers an allocation failure during marking by requesting `FinalMark` early (`src/plan/concurrent/immix/global.rs:144-148`). That pause then finishes all the remaining marking stop-the-world.
- Heap exhaustion is handled by **allocation stalls** (§5.11): the allocating thread waits for the concurrent cycle, which runs with every GC worker.
- **`System.gc()`** (`handle_user_collection_request`) starts a concurrent cycle if none is running, and the calling thread stalls until the cycle completes. That is what Go's `runtime.GC()` does.
- **Emergency collections** (MMTk's last attempt before out-of-memory, which must clear soft references) are concurrent cycles too, with the "clear all soft references" policy (§5.11).

`Pause::Full` (STW mark + eager sweep) remains only for phases 1 and 2, before concurrent marking exists, and behind a debugging option `full_gc_fallback` (off by default).

**Go-style mark termination.** `ConcurrentImmix`'s FinalMark finishes whatever marking is left (`FinishConcurrentWork`, then the closure) inside the pause. Go's mark termination (`gcMarkDone` + `gcMarkTermination`, `mgc.go:997-1531`) does no tracing at all. `ConcurrentSpan` does it Go's way, in two steps:

1. **Concurrent termination detection** (Go's `gcMarkDone` loop). When the `Concurrent` bucket drains, the plan does not request FinalMark yet. Its `on_concurrent_work_drained` adds a `RaggedFlush` packet instead. That packet first sets the **weak→strong block** (§5.14), then asks every mutator to flush its barrier buffer, through a handshake (§5.9), and records whether any buffer held work. The handshake also makes every thread see the block, as `forEachP` does in Go (`mgc.go:1018-1022`).
   - If a round found work, tracing continues. When the bucket drains again, another round runs.
   - When a round finds no work, the plan requests FinalMark. That is Go's `gcMarkDoneFlushed == 0`.
2. **The FinalMark pause does no tracing** (Go's mark termination STW).
   - Its first packet flushes every mutator's barrier buffer again. Mutators are stopped, so this sees everything. If any entry is an unmarked object, the pause **restarts concurrent marking**: the work goes into `Concurrent`, the barrier stays on, and no further pause packets are scheduled. The scheduler already resumes mutators into concurrent work when `Concurrent` is non-empty at the end of a pause (`schedule_concurrent_packets`). That is Go's "restart the world and resume concurrent mark".
   - Otherwise the packet schedules the rest of the pause: reference processing that needs no tracing (§5.14), disabling the barrier and allocate-black, `sweepgen += 2`, the pacer commit, waking stalled mutators, and clearing the weak→strong block and waking the threads it blocked (`mgc.go:1137-1138`).
   - The block stays set if marking restarts, as in Go, so it is only cleared once marking is really done.

So the pause does no marking. Its work is a buffer check, list-based reference processing, VM weak roots, and bookkeeping. Restarts and ragged rounds are counted and reported (§10).

**Why keep a pause at all.** Go keeps this STW, and so do we, for fidelity. Removing it is possible in principle: flip a thread-local barrier flag per thread in a handshake, change epochs without stopping threads, and process references concurrently with a `Reference.get()` barrier that returns null for unmarked referents, as ZGC and Shenandoah do. That goes beyond Go, and would change MMTk's reference processing, so it is out of scope (§12).

### 5.3 Heap: `SpanSpace`

**Why a new policy instead of extending native `MarkSweepSpace`.**

| Concern | `MarkSweepSpace` today | Go's design (adopted) |
|---|---|---|
| Block/span size | Fixed 64 KiB blocks | 8 KiB pages, 1–10-page spans per class |
| Size classes | 48 mimalloc bins up to 64 KiB | 67 Go classes up to 32 KiB |
| Free cells | Free list threaded through dead cells | `allocBits` bitmap + 64-bit `allocCache` with ctz. No writes into dead objects |
| Mark bits | VM `LOCAL_MARK_BIT_SPEC`, cleared at Prepare | Per-span bitmaps indexed by object index, swapped at sweep, never cleared |
| Sweep state | `BlockState::{Unallocated, Unmarked, Marked}` | `sweepgen` CAS protocol |
| Central | One mutex-guarded `abandoned` list | Per-class lock-free span sets |
| Scan/noscan | None | Separate span classes |
| Object index | Not needed (free list) | Page map + magic divide, O(1) |

Retrofitting all of that would change `MarkSweepSpace` for its current users (MarkSweep and `marksweep_as_nonmoving`). A separate policy keeps them stable. Phase 1 (§8) still starts on `MarkSweepSpace` to get the plan running, then swaps in `SpanSpace`.

**Pages and chunks.**
- `SpanSpace` uses Go's 8 KiB page as its internal unit. That is 2 MMTk pages.
- It owns a page allocator over 4 MiB chunks, which is MMTk's chunk size and also Go's `pallocChunk`. Each chunk has a 512-bit in-use bitmap and a first-fit search, as in `mpagealloc.go`.
- Phase 2 can use a linear scan over chunks. The radix-tree summaries are an optimisation for later.
- Chunks come from `Space::acquire`, so the VMMap, SFT, mmapper and `get_used_pages` keep working.
- Open question: which page resource hands out chunk-aligned 4 MiB units. The candidates are `BlockPageResource` with a chunk-sized `Block`, or a `FreeListPageResource` alignment extension.
- Spans that are one page (≤1280 B classes) must be 8 KiB-aligned, because Green Tea's inline marks rely on `base + PageSize`.

**Span descriptors.**
- `Span` is a Rust struct stored off-heap in a growable, index-stable table. Go uses `fixalloc` the same way.
- Fields: `start`, `npages`, `spanclass`, `nelems`, `elemsize`, `div_magic`, `freeindex`, `alloc_cache`, `alloc_bits`, `mark_bits`, `sweepgen`, `state`, `needzero`, `owner`.
- Bitmaps are per object index. They are allocated from a `GcBits` arena with the same current/next/free epoch rotation as `gcBitsArena` (`mheap.go:2871-2913`).
- This does not use MMTk side metadata for marks. Address-granular side metadata would cost 1 bit per 8 bytes and make the sweep walk granules, not objects.
- Side metadata is used for the **page map**: one `u32` span id per 8 KiB page, a new local spec. That is 1/2048 overhead. It is the `heapArena.spans` equivalent and the basis of `trace_object`.

**Size classes.**
- `sizeclasses.rs` is generated by a small script that runs Go's `_mkmalloc/mksizeclasses.go` and emits Rust tables: size, npages, divMagic, and the two size→class lookup tables.
- Keeping it generated, not hand-copied, makes Go's waste guarantees hold by construction and makes upstream changes easy to follow.
- Java objects are at least 16 bytes and 8-byte aligned, so Go's 8-byte class is unused on OpenJDK. That costs nothing.
- `max_non_los_default_alloc_bytes = 32768`. Larger objects go to the common LOS, which plays the part of Go's class-0 spans. mmtk-openjdk already sends allocations above the plan's `max_non_los_default_alloc_bytes` to the slow path and the LOS.

**Scan vs noscan.**
- Go segregates pointer-free objects so the marker can blacken them without scanning.
- mmtk-core's `alloc` has no such flag. We propose a new `AllocationSemantics::NoScan`.
- It maps to a second `SpanAllocator` instance in `ConcurrentSpan` and to `Default` in every other plan, so it is always safe to use.
- `trace_object` on a noscan span marks the object and does not enqueue it.
- **mmtk-openjdk does not use it.** Every Java object goes to a scan span. Primitive arrays and reference-free instances are still enqueued and passed to `scan_object`, which finds no fields. The semantic exists so the core matches Go and can be tested on MockVM, and for other bindings.

**Allocator (`SpanAllocator`, the mcache).**
- Fast path: `nextFreeFast` over `alloc_cache`, using ctz.
- Slow path:
  1. Refill `alloc_cache` from `alloc_bits`.
  2. When the span is full, uncache it and take one from the class's central: partial-swept, then partial-unswept (sweep it), then full-unswept (sweep it), then grow.
  3. Set `sweepgen = sg + 3`.
- Refill is also where Go charges `heapLive`. It over-counts by the span's free bytes. Refill is the pacer poll point (§5.10) and the assist debit point (§5.11).
- Zeroing: a span carries `needzero`. Fresh pages from the mmapper are zero, so only reused spans zero on allocation, as in Go.
- **OpenJDK fast path.** Until mmtk-openjdk emits the `alloc_cache` ctz fast path in its x86_64 and aarch64 assemblers and C2, every allocation calls into MMTk (gap 9). That is correct but slow, so functional phases can run without it, but performance numbers need it. It needs an `AllocatorInfo` variant for `SpanAllocator` in mmtk-core. **Status:** done for x86_64 (interpreter, C1, C2) using `AllocatorInfo::Span` (with `space_offset`/`allocate_black_offset` for the allocate-black bail-out) and `span_allocator::span_class_index`; aarch64 still takes the slow path. Decisions D80-D87, deviations FP-1..FP-5.

**Object references.**
- OpenJDK references always point to the object start, and mmtk-openjdk uses no allocation offset.
- `trace_object` only needs the object-index computation (§5.4). No interior-pointer resolution is involved.

### 5.4 Mark state: epochs and `sweepgen`

`SpanSpace` keeps a global `sweepgen`, advanced by 2 in FinalMark's release. Mark bits follow Go exactly:
- **Allocation epoch:** when a span is swept, `alloc_bits = mark_bits` and a fresh zeroed `mark_bits` is taken from the next `GcBits` epoch.
- **No Prepare-time clearing.** This replaces `PrepareChunkMap` and is what removes the hazard in gap 1: a span's marks for cycle *n* only exist after it was swept for cycle *n−1*.
- **Invariant enforced at InitialMark:** every in-use span has `sweepgen == h` before marking starts (§5.5).
- `trace_object(obj)`:
  1. Page map lookup gives the span.
  2. `idx = ((obj - start) * div_magic) >> 32`.
  3. Atomic OR of the bit in `mark_bits`.
  4. If newly marked and the span class is scan, enqueue the object, or the span under Green Tea (§5.12).
  5. Add `elemsize` to a per-worker `bytes_marked`, which feeds `heapMarked` (§5.10).
- `is_live` / `is_reachable` read the same bit. `SpanSpace` never moves, so `trace_object` returns `obj`, and `pin_object` is trivially true.

### 5.5 Sweeping

Three sweepers share the `sweepgen` CAS (`h-2 → h-1`) as in `mgcsweep.go:342`:

1. **Allocator sweeping.** Mutators sweep unswept spans they pick up from central, as above. This is MMTk's existing lazy-sweep idea, now made safe by the epoch scheme.
2. **Background sweeping.** After FinalMark, `on_pause_end` puts `SweepSpans` packets (one per N chunks) into the `Concurrent` bucket. The scheduler then enters `GcStatus::InConcurrentGC` with `concurrent_threads` workers.
   - When the bucket drains, `on_concurrent_work_drained` calls `Collection::concurrent_work_finished_no_pause`, as LXR does.
   - This reuses existing scheduler support. The only plan-level work is making sure an InitialMark request that arrives while sweeping is still running interrupts it cleanly. `on_concurrent_work_interrupted` must leave the remaining sweep packets for the InitialMark pause.
3. **Sweep termination.** The previous cycle must be fully swept before InitialMark. Go makes the triggering goroutine sweep the leftovers *before* the STW pause, so we do the same: when a cycle is triggered while sweep packets remain, the plan runs them first, concurrently and with all workers active, and only then requests InitialMark. The InitialMark pause still runs any residue in an STW stage ahead of `Prepare` as a safety net, asserts that all spans are swept, and flips the `GcBits` epoch. Go's pacer sizes sweep credit so this residue is normally empty. Its size is a direct pause-time metric (§10).

**Proportional sweep** (`deductSweepCredit`, `mgcsweep.go:920`) is reproduced in the allocator slow path. On refill, a mutator sweeps `sweepPagesPerByte × allocated` pages before taking a span. This is cheap to add once (1) exists.

**Sweeping a span:**
- Swap the bitmaps.
- Count free slots.
- If the span is empty, return it to the page allocator. Otherwise push it to the class's partial or full swept set.
- Zombie detection (marked but free) is a `debug_assertions` check.

### 5.6 Write barrier: `HybridBarrier`

`ConcurrentSpan` has its own barrier, a port of Go's (§3.2), selected by a new `BarrierSelector::HybridBarrier`:

```
object_reference_write_pre(src, slot, target):
    if !MARKING_ACTIVE (a global byte the JIT tests inline):
        return
    buf.push(slot.load())     // deletion (Yuasa): old value
    buf.push(target)          // insertion (Dijkstra): new value
    if buf.full(): flush()    // -> ProcessHybridBuf packet into Concurrent (or Closure at FinalMark)

memory_region_copy_pre(src, dst):        // arraycopy
    if marking: push every old pointer in dst and every pointer in src

object_clone(src, dst):                  // Object.clone
    if marking: push every pointer copied into dst
```

Like Go's barrier, it does not implement the "only if the current stack is grey" condition, so it always logs both values (`mwbbuf.go:221-227`). A per-thread `stack_scanned` check could add it later.

**How it differs from `SATBBarrier`:**
- It has no unlog bit and needs no log-bit metadata. It is flag-gated per write, not once per object.
- It records old **and** new values.
- The flush packet ignores nulls and values outside MMTk spaces, then calls `trace_object`. That is `wbBufFlush1`.

**Flag.** The flag lives in `ConcurrentSpan` and is exported through a new `memory_manager::barrier_active_address()`, so compiled code can test one byte, like Go's `writeBarrier.enabled`. `on_pause_end(InitialMark)` sets it. `on_pause_start(FinalMark)` clears it after flushing every mutator's buffer. Changing it inside an STW pause gives the same visibility guarantee as Go's `setGCPhase`.

**Buffer.** 512 entries per mutator, Go's `wbBuf` size, configurable by option. The buffer's cursor and limit sit in the MMTk mutator, so compiled code can append inline and only call into MMTk when the buffer is full.

**OpenJDK requirements.** Everything that can make a heap object reachable from somewhere the marker has already looked must be barriered:
- **Heap reference stores** in the interpreter, C1 and C2: inline flag test and buffer append, with a runtime slow path. This is new code in mmtk-openjdk, modelled on `MMTkSATBBarrier`, for x86_64 and aarch64.
- **`arraycopy` and `Object.clone`**: Go's `bulkBarrierPreWrite` equivalent, in the runtime, C1 and C2 arraycopy stubs and the clone intrinsic.
- **`Reference.referent` loads**: logged while marking, as the SATB barrier already does. That is the Java counterpart of Go shading the target in `weak.Pointer.Value()`. During mark termination these loads block instead (§5.14).
- **Stores into VM roots** (OopStorage, such as JNI global handles): VM roots are scanned once, in the InitialMark pause. A reference stored into one later, from a stack that has not been scanned yet, must be shaded, or it can be lost when the stack drops it. These stores all go through the C++ access API (`oop_store_not_in_heap`), so mmtk-openjdk shades the new value there. Go needs no equivalent because its globals are ordinary barriered memory.
- **nmethods registered during marking**: their embedded oops are shaded at registration. That is the insertion half applied to code roots.
- Stack writes are not barriered, as in Go.

**SATB option.** `ConcurrentSpan` can also run with `SATBBarrier` (option `concurrent_span_barrier = satb`), the barrier `ConcurrentImmix` uses. That is for comparison runs: it isolates the barrier's cost. It needs log-bit support in `SpanSpace`:
- `SpanSpace` implements `set_side_log_bits` / `clear_side_log_bits` per allocated chunk, the way native `MarkSweepSpace` does (`native_ms/global.rs:258-270`). The plan bulk-sets unlog bits at InitialMark and clears them at FinalMark.
- Objects allocated during marking are left *logged*, so the barrier ignores them. They are already black. `ImmixAllocator` does the same (`src/util/alloc/immix_allocator.rs:272-281`).
- With SATB, stacks must be scanned in the InitialMark pause or through `StackWatermark` (§5.7). The handshake mode is not sound without the insertion half.

The barrier is chosen when the plan is created, so `ConcurrentSpan` returns one of two static `PlanConstraints` (§7).

### 5.7 Roots and stack scanning

Go scans stacks concurrently, one goroutine at a time. mmtk-core scans mutator roots only while all mutators are stopped. `ConcurrentSpan` supports three modes, chosen by a plan option `stack_scan`:

| Mode | Stacks scanned | Barrier needed | Use |
|---|---|---|---|
| `initial_mark` | All, in the InitialMark pause | Either | Phase 3, and the baseline |
| `handshake` (default) | Each thread once, during marking, in a per-thread handshake | `HybridBarrier` | Go-faithful |
| `watermark` | Each thread as of InitialMark, lazily and concurrently, via `StackWatermark` | Either | Comparison: ZGC/Shenandoah-style stack processing |

**`initial_mark` mode (phase 3).**
- All mutator roots are scanned in the InitialMark pause using the existing `ConcurrentMarkingRootsWorkFactory` path.
- Root slots are loaded in the pause, and tracing is concurrent.
- Pause time grows with total stack size. That is the main deviation from Go, and the reason for the other modes.

**`handshake` mode (phase 4, Go-faithful).** This is Go's scheme: each stack is scanned once, at some point during marking, while only that thread is stopped. After a thread's stack is scanned, it is black. `HybridBarrier` makes this sound, exactly as in Go: the insertion half catches a thread that moves its only reference to an unmarked object from its unscanned stack into an already-marked object.

On OpenJDK it uses `Handshake::execute(cl)`:
- It runs `cl` on every Java thread **without a safepoint**. A running thread runs the closure itself at its next poll and stops only for its own closure. For a thread that is blocked or in native code, the VM thread runs the closure on its behalf while the thread cannot resume.
- A GC thread can call it, as ZGC's GC threads do. mmtk-openjdk's GC workers are `NamedThread`s.
- The closure walks the thread's stack (`thread->oops_do`, as `scan_roots_in_mutator_thread` does now), reports the references it finds, and marks the thread scanned. Tracing happens later, in GC workers.
- The cost on each thread is the stack walk only. That matches Go, where `scanstack` runs while the goroutine is suspended.

Go's workers suspend goroutines one at a time and scan them themselves. The closest HotSpot form is the single-thread synchronous handshake, but that needs a Java thread as the caller. mmtk-openjdk could add a binding-owned Java thread for it (it already has one, `MMTkFinalizerThread`). That would let blocked threads be scanned in parallel by several requesters instead of serially by the VM thread. It is an open question (§12).

**`watermark` mode (optional).** HotSpot's `StackWatermark` processes each thread's stack *as it was when the GC epoch started*, lazily and concurrently. ZGC and Shenandoah use it, and Shenandoah's concurrent mark processes Java thread stacks this way with only an SATB barrier (`ShenandoahConcurrentMarkThreadClosure`, `gc/shenandoah/shenandoahRootProcessor.cpp:117`). It is sound with either barrier:
- Frames that existed at the epoch change are processed before the thread can run in them again. Frames created later only hold references loaded from the heap or older frames, or new objects.
- The GC registers a watermark per Java thread (`StackWatermarkSet::add_watermark`, from `BarrierSet::on_thread_attach`, as `shenandoahBarrierSet.cpp:113` does). Starting a new epoch makes each thread process its top frames at its next poll. Returning into an unprocessed frame traps at the return poll, which the interpreter, C1 and C2 already emit. A GC thread processes the rest with `StackWatermarkSet::finish_processing`.
- The per-thread pause covers only the frames a thread touches, which is shorter than Go's.

This mode is useful to compare Go's stack scanning with ZGC/Shenandoah-style processing, and to give `ConcurrentImmix` concurrent stack scanning with its existing SATB barrier.

**mmtk-core API (VM-neutral, reusable by `ConcurrentImmix`).**
- A new binding capability `Scanning::CONCURRENT_STACK_SCAN_MODES`, default none. A mode only takes effect if the binding supports it.
- `Scanning::start_concurrent_stack_scan(tls, mode)`, called in the InitialMark pause instead of creating `ScanMutatorRoots` packets. In `watermark` mode, mmtk-openjdk starts a new epoch. In `handshake` mode it does nothing.
- `Scanning::scan_stacks_concurrently(tls, factory)`, called from one `ConcurrentScanStacks` packet in `Concurrent`. It returns once every thread's stack has been scanned. Slots reported on the GC thread go to `factory`, which creates `Concurrent` tracing packets.
- `memory_manager::report_stack_roots(mutator, &[ObjectReference])`, for references found by a thread scanning its own stack (in a handshake closure or a watermark trap). They are appended to that mutator's barrier buffer, which already holds "objects that must be marked", and are flushed into tracing packets as usual.
- Mutators bound during marking start as already scanned. That is Go's "new goroutines are black".
- The `ConcurrentScanStacks` packet is in the `Concurrent` bucket, so the bucket only drains, and FinalMark is only requested, after every stack has been scanned. FinalMark does **not** rescan stacks, as in Go and `ConcurrentImmix`.

**What stays in the InitialMark pause.** VM roots (`scan_vm_specific_roots`): OopStorage roots such as JNI handles, class loader data, and mmtk-openjdk's code-cache root set. Go scans its globals concurrently because they are ordinary barriered memory. Here, VM roots are loaded in the pause, and later stores into them are shaded by the barrier (§5.6). Processing OopStorage concurrently, as Shenandoah does, is a later option (§12).

### 5.8 Allocate-black

While `allocate_as_live` is set (InitialMark end to FinalMark start), `SpanAllocator::post_alloc` marks the object (`mark_bits` OR) and adds `elemsize` to `bytes_marked`. That is `gcmarknewobject`.

Go's size-specialised malloc fast paths bail out to the slow path whenever marking is on (`malloc_stubs.go:82`). We do the same: the inline fast path checks the flag, and the slow path does the marking. That keeps the common fast path branch-light.

LOS already honours `allocate_as_live`.

### 5.9 Marking work, workers and termination

- **Tracing packets.** `SpanTraceObjects` is modelled on `ConcurrentTraceObjects`: a DFS with a local deque, splitting overflow into new packets. Two additions:
  1. per-packet `scan_work` and `bytes_marked` counters, flushed to the pacer and to background credit (§5.11);
  2. 128 KiB oblet splitting for large objects. This needs `Scanning::scan_object_range` or an equivalent "scan from offset" hook. Without it, a huge array is one indivisible task. On OpenJDK this matters for large reference arrays, and mmtk-openjdk can scan an `objArray` sub-range.
- **Background utilisation.**
  - Go runs `round(0.25 × GOMAXPROCS)` dedicated workers, plus fractional and idle workers.
  - mmtk-core's `concurrent_threads` option already defaults to `threads/4` (`src/util/options.rs:369`), so dedicated workers map directly.
  - Fractional and idle workers have no MMTk equivalent because MMTk workers are OS threads, not scheduled on Ps. The OS gives idle cores to runnable workers. Assists add workers when allocation outpaces marking (§5.11).
- **Termination** (§5.2).
  - The `Concurrent` bucket drains and the last parked worker calls `on_concurrent_work_drained`. Instead of requesting FinalMark directly, `ConcurrentSpan` runs ragged flush rounds until one finds no work.
  - **Ragged flush.** A new `Collection::flush_mutators_concurrently(tls) -> bool` asks every mutator to flush its barrier buffer, like Go's `forEachP`, and returns whether any buffer held work. mmtk-openjdk implements it with `Handshake::execute(cl)`, the same non-safepoint handshake as stack scanning (§5.7). A blocked thread's buffer is flushed by the VM thread on its behalf. (An asynchronous handshake would not work: only the target thread runs it, so a blocked thread would never answer.)
  - The FinalMark pause checks the buffers once more and restarts concurrent marking if it finds work.

### 5.10 Pacer: `FeedbackPacer`

`plan/concurrent/span/pacer.rs` is a pure-arithmetic port of `gcControllerState`: `startCycle`, `revise`, `endCycle`, `commit`, `heapGoalInternal`, `memoryLimitHeapGoal` and `trigger`. It has no MMTk dependencies, so it can be unit-tested directly against Go's `TestGcPacer` scenarios (`mgcpacer_test.go:16`, `gcExecTest` at `:759`). Porting those scenarios is the pacer's acceptance test.

It is wired in as `GCTriggerSelector::FeedbackPacer`, with new options `gc_percent` (default 100, `off` allowed) and `soft_mem_limit`.

**Heap limits on OpenJDK.** `-Xmx` is a hard limit, which Go does not have.
- `soft_mem_limit` defaults to the maximum heap size when one is set. The memory-limit goal (with Go's headroom) then keeps cycles finishing below `-Xmx`, the way `GOMEMLIMIT` keeps Go below its limit.
- If the heap still reaches `-Xmx`, the allocating thread stalls until the cycle frees memory (§5.11). There is no STW full GC.

**Inputs and where they come from:**

| Pacer input | Source in MMTk |
|---|---|
| `heapLive` | Per-mutator counter charged at span refill, as in Go's `mcache.refill`. Corrected at uncache. LOS allocations are charged directly. *Not* `get_used_pages`, which is chunk-granular for `SpanSpace` |
| `heapMarked` | Sum of per-worker `bytes_marked`, including allocate-black, collected at FinalMark |
| `heapScan` | Bytes in scan spans at mark start, maintained by the allocator |
| `stackScan` / `globalsScan` | Reported by root scanning (bytes of stack and VM roots scanned). A new `Collection::scannable_stack_bytes()` estimate feeds `maxStackScan`. mmtk-openjdk can sum each thread's used stack |
| Scan work done | `SpanTraceObjects` counters |
| Assist and worker time | Timestamps around packets, and time mutators spend parked for assists (§5.11) |
| `mappedReady`, `heapFree` (memory limit) | `SpanSpace` page allocator stats plus `Collection::vm_live_bytes()` for off-heap |

**Trigger point.**
- `SpanAllocator` refill and LOS allocation call `gc_trigger.poll(false, Some(space))` when `heapLive >= trigger`.
- `GCTrigger::poll` only *requests* a pause and does not block (`src/util/heap/gc_trigger.rs:146-173`). The allocating thread continues and is stopped at its next safepoint, which matches Go.

**Heap size reporting.** `FeedbackPacer` reports `get_current_heap_size_in_pages = goal` and `can_heap_size_grow = true`, capped by the maximum heap size.

**CPU limiter.** The leaky bucket in `mgclimit.go` is ported with the pacer. It only gates assists (§5.11), so it costs little.

### 5.11 Mutator assists and allocation stalls

Go makes an allocating goroutine that runs ahead of marking do mark work itself. That does two things: it slows the goroutine's allocation, and it adds marking throughput, because the goroutine's CPU time goes to marking.

**Stage A: park and lend the CPU to an extra worker (phase 3).**
- Each mutator keeps `assist_bytes`, debited by allocation at refill. Go debits per object; refill granularity is coarser but bounded by span size.
- When it goes negative during marking, the mutator first tries to steal from global `bg_credit`. Workers add to `bg_credit` as they finish packets: scan work divided by `assistWorkPerByte`.
- If credit is still insufficient, the mutator parks on an assist queue, **and the plan activates one more concurrent worker** while it is parked. The cap becomes `concurrent_threads + parked_assists`, using `WorkerMonitor::set_active_workers` and its existing notification. A worker only checks the cap when it parks, so a worker that loses its slot finishes its current packet first.
- Credit flushes wake parked mutators in FIFO order, as in `gcFlushBgCredit`. The extra workers' work goes into `bg_credit` like any other worker's.
- This gives both effects of a Go assist without running GC packets on mutator threads. It deviates from Go in that the assist runs on a different thread (§11).

Rules that must hold:
- **Release everyone when marking ends.** When the `Concurrent` bucket drains or FinalMark is requested, all assist-parked mutators are woken and their debt forgiven, as Go's `gcWakeAllAssists` does. Otherwise FinalMark waits for a mutator that waits for credit that never comes. `parked_assists` is reset to 0 at the same time, so late wake-ups cannot push the cap below `concurrent_threads`.
- **Parking must be GC-safe.** The parking primitive is a new optional `Collection::park_for_assist(tls)` / `unpark`. mmtk-openjdk implements it like `block_for_gc`: the thread transitions to a blocked state, so safepoints, handshakes and `finish_processing` of its stack can proceed while it is parked. A default `Condvar` implementation works for MockVM.
- **No spare workers.** The cap is clamped to `threads`. If all workers are already active, assists fall back to parking only.
- **Cost.** Each assist costs a park and a worker wake-up, where Go assists run in-thread. The minimum assist size (Go: 64 KiB) can be raised to amortise that.
- Assists are suppressed while the CPU limiter is limiting, and the extra workers' time counts as assist time for the limiter and the pacer. Credit flushes happen every 2000 scan-work units (`gcCreditSlack`).

**Allocation stalls: heap exhaustion without a full GC.** An assist is a mutator paying for marking before the heap runs out. A stall is the limit of that: the heap *has* run out, so the mutator waits for the GC, and its CPU goes to the GC. It reuses Stage A's parking and worker activation:
- **Where.** When `Space::acquire` cannot get pages (`src/policy/space.rs:262-285`), it calls `block_for_gc` today, which in mmtk-openjdk returns at the end of the *next pause*. For a concurrent cycle, that is the end of InitialMark, which has freed nothing. `ConcurrentSpan` instead takes over this path through a new plan hook, and the mutator stalls.
- **What happens.**
  1. If no cycle is running, the plan triggers one, which runs any leftover sweeping first (§5.5).
  2. The mutator parks with `Collection::park_for_assist`, which is GC-safe (Stage A above), so handshakes and the InitialMark and FinalMark pauses proceed while it waits.
  3. While any mutator is stalled, the worker cap is raised to **all** GC workers (`threads`), not just one extra per mutator. The application cannot use its CPU anyway.
  4. The cycle runs to completion. FinalMark is requested when `Concurrent` drains, as usual.
  5. At the end of the FinalMark pause, `sweepgen` has advanced and spans can be swept. Stalled mutators are woken and retry. Their own allocator sweeping (§5.5) and the background sweepers find free spans.
- **Out of memory.** If a mutator still cannot allocate after a complete cycle, the plan runs another cycle, as MMTk's collection-attempt counting does today (`GlobalState::determine_collection_attempts`, `src/global_state.rs`). The last attempt is an emergency cycle with "clear all soft references", as Java requires before `OutOfMemoryError`. If that also fails, MMTk reports out of memory as it does now.
- **Precedent.** ZGC does the same in HotSpot: an allocation that cannot be satisfied stalls until the concurrent cycle frees memory (`ZPageAllocator::alloc_page_stall`, reported as "Allocation Stall"). It has no STW full GC.
- **Large objects.** LOS allocation failures take the same path.
- **What can still block.** A stalled thread may hold Java monitors, so other threads may block on them. That does not stop the GC: marking, stack scanning and the two short pauses never wait for a mutator to make progress.

Stalls are a fallback. The pacer's job (§5.10) is to finish each cycle before the heap runs out. Stall count and stall time are reported next to pause times (§10), because a collector without long pauses can still make the application wait.

**Stage B: real assists (optional, last).**
- The mutator executes mark work itself through `memory_manager::assist(mutator, scan_work_budget)`. On OpenJDK that means Java threads running MMTk work from the allocation slow path.
- This needs:
  1. a per-mutator lightweight worker context (local queue, tracer) that is not a `GCWorker`;
  2. a way to take packets from `Concurrent` without becoming a worker (`GCWorkScheduler::try_take_concurrent_packet`);
  3. `SpanTraceObjects` honouring a scan-work budget and pushing back what it has left.
- It changes the scheduler's ownership model, because packets would run on non-worker threads. That makes it an MEP-level change. We only do it if Stage A's measurements show it is needed.

### 5.12 Green Tea span scanning (phase 6)

Green Tea is plan-local (`src/plan/concurrent/span/greentea.rs`, `src/policy/spanspace/span_queue.rs`), and on by default. The option `concurrent_span_green_tea = false` turns it off for ablation, which gives exactly the object-queue marking of phase 4.

`SpanTraceObjects` bypasses the generic `ObjectQueue`, so it holds two queues:
- a span queue: a local FIFO ring of 256 per packet. As in Go's `spanQueue.put`, it spills half of the ring when full, and up to 16 spans every 64 puts when the last spill has been taken. Spills become new `SpanTraceObjects` packets, which other workers take from MMTk's bucket or steal from the worker's local packet buffer (deviation GT-3);
- an object queue (LIFO), for larger objects and other spaces, with the same overflow splitting as `ConcurrentTraceObjects`.

It takes local objects first, then local spans, as `gcDrain` does. All concurrent marking packets of the plan (roots, barrier buffer flushes, rescans, finalizable contents, and the work `FinalMark` finishes in the pause) are `ConcurrentTraceObjects` or `ProcessModBufSATB` packets that hand their objects to `SpanTraceObjects` through `ConcurrentPlan::trace_objects_concurrently` (decision D71). Stop-the-world `Full` pauses and reference processing keep the generic trace (GT-6).

For `SpanSpace` one-page spans with 16–512 B scan objects (`SpanClass::uses_span_queue`):
- `trace_object_for_span_queue` sets the object's mark bit, then tries to acquire the span's `owned` byte with Go's `tryAcquire`. If that succeeds, it pushes the span (with the object's index, for Go's one-mark fast path).
- `scan_span` releases `owned` first, then scans `marks &^ scans` and sets those scan bits. A mark that raced with the scan is either seen by it or queues the span again.
- Allocate-black sets the scan bit before the mark bit, so new objects are never scanned. The generic `trace_object` sets the scan bit of an object it enqueues.
- The scan bits and `owned` are in the span descriptor, next to the mark bits, not in an inline block at the span page tail (deviation GT-1, decision D70). Sweeping clears them. There is nothing to OR into `mark_bits` at sweep, because the marks already are `mark_bits`.

**Object scanning.** Go's span scan reads a span-tail pointer bitmap directly, with SIMD for dense spans. OpenJDK has no per-span pointer bitmap: reference fields are described per class (oop maps). We call `Scanning::scan_object` per marked object. That keeps Green Tea's locality and load-balancing benefit, without the SIMD path. Building pointer bitmaps for OpenJDK objects at allocation is out of scope.

**Layout cost.** Go's Green Tea reserves 128 B for inline marks in each one-page span, and `initSpan` (`mheap.go:1458-1476`) computes `nelems` from the remaining bytes. With the state in the descriptor, `SpanSpace` keeps the generated table's object counts (GT-1).

**Object references.** A span scan finds objects by index, so it has to turn a cell into an object reference. `SpanSpace` learns the offset of object references from their cells from the first object it marks for the span queue, and asserts that it never changes (0 in OpenJDK). Spans with padded cells (an allocation offset, never used by OpenJDK) use the object queue (decision D73).

### 5.13 Object index lookup

`trace_object` needs the object index of a reference, which `SpanSpace` computes in O(1):

1. Page map lookup gives the span.
2. Check that the span is in use.
3. Divide by `div_magic` to get the object index.

Go uses the same lookup to resolve interior pointers. Java references always point to the object start, so `ConcurrentSpan` adds no interior-pointer API. The lookup could later serve a binding that scans conservatively, but that is out of scope.

### 5.14 Weak references and finalizers

`ConcurrentSpan` uses MMTk's mechanism, not Go's specials:
- **Discovery.** The binding registers reference objects with `memory_manager::add_{soft,weak,phantom}_candidate` and finalizable objects with `add_finalizer`. mmtk-openjdk already does both, for every plan.
- **Processing.** In the FinalMark pause, the plan schedules the same packets as `ConcurrentImmix::schedule_concurrent_marking_final_pause` (`src/plan/concurrent/immix/global.rs:429-479`):
  - `SoftRefProcessing`, `WeakRefProcessing` and `PhantomRefProcessing` in the reference-closure buckets, then `RefEnqueue` in `Release`;
  - `Finalization` in `FinalRefClosure`;
  - `VMProcessWeakRefs` as the `VMRefClosure` sentinel.

  As in `ConcurrentImmix`, these honour the `no_reference_types` and `no_finalizer` options.

**Keeping the pause free of tracing.** Today two of these packets trace in the pause. `SoftRefProcessing` keeps soft referents alive by tracing from them, unless the collection is an emergency (`src/util/reference_processor.rs:548-570`). `Finalization` traces everything reachable from resurrected finalizable objects (`src/util/finalizable_processor.rs`). Go-style mark termination (§5.2) must not trace, so both move into concurrent marking:
- **Soft references.** Whether this cycle clears soft references is decided at InitialMark. In a normal cycle, soft referents are treated as strong and traced during concurrent marking: the binding asks a new `memory_manager::soft_references_are_strong(mmtk)` when it scans a `SoftReference`, and reports the referent as an ordinary slot instead of calling `add_soft_candidate`. In an emergency cycle they are discovered as candidates and cleared, which needs no tracing.
- **Finalizers** (Go's `gcScanFinalizer`). During concurrent marking, a packet scans the *contents* of every registered finalizable object, without marking the object itself. At termination, `Finalization` only marks the unreachable candidates (resurrects them). Their contents are already marked, so no closure is needed. This keeps exactly the same objects alive as today: if the finalizable object is reachable, its contents are traced anyway, and if it is not, it is resurrected with everything it references. Finalizable objects allocated during marking are allocated black, and their fields are written through the barrier.
- **Weak and phantom references** are cleared or kept by looking at mark bits. That is list processing, proportional to the number of references, with no tracing.
- **VM weak roots** (`VMProcessWeakRefs`, such as HotSpot's string table) are processed in the pause, proportional to their size. Processing them concurrently, as Shenandoah does, is a later option.

Both changes are extensions to MMTk's reference and finalizer processing, usable by `ConcurrentImmix` too.
- **Emergency cycles** use the same packets with the "clear all soft references" policy (§5.11). `Pause::Full`, where it is still used (§5.2), uses them through the usual STW path.
- **Weak loads during marking.** `Reference.get()` keeps its referent alive because the barrier logs `Reference.referent` loads while marking (§5.6), with either barrier. That is Go's `shade(ptr)` in `makeStrongFromWeak` (`mheap.go:2459-2466`).
- **Weak→strong block during mark termination.** Go blocks `weak.Pointer.Value()` from the start of mark termination until marking is done (`work.strongFromWeak.block`, `mheap.go:2416` and `:2478-2487`). Otherwise each conversion could create new marking work after termination detection has started, and termination might not converge. `ConcurrentSpan` does the same for Java's weak→strong conversions:
  - **What blocks.** `Reference.get()` on soft, weak and phantom references (the interpreter, the C1 and C2 intrinsics, and the runtime), and resolving a JNI weak global reference into a strong one. `Reference.refersTo()` does not block: it creates no strong reference.
  - **How.** These loads already take the barrier's weak-load path while marking. In that path, if the block flag is set, the thread calls a new `memory_manager::wait_for_weak_to_strong(mutator)`. It parks GC-safely (the same parking as assists, §5.11) until the flag is cleared, then the thread loads the referent again. By then reference processing has run, so the result is either a live object or null.
  - **When.** The flag is set by the first ragged flush round (§5.2), stays set across restarts, and is cleared at the end of the FinalMark pause, after reference processing. Then the blocked threads are woken, and they run when the pause ends. Go clears it in the same stop-the-world pause, once marking is confirmed done (`mgc.go:1137-1138`). Go handles weak handles later, during sweeping, and a woken goroutine calls `ensureSwept` before it reads the handle. We process references inside the pause instead, so a woken thread reads an already-processed referent.
  - **VM-internal weak loads do not block.** HotSpot resolves weak handles inside the VM, for example in the string table and the resolved-method table, sometimes while holding VM locks. Parking there risks deadlock, so those loads only shade, as outside the termination window. That can create work during termination, which at worst costs another ragged round or a restart.
  - Like Go, the block does not look at the referent first. Returning immediately when the referent is already marked would be safe and would shorten waits, but it is a deviation, so it is an option (`weak_to_strong_block = always | unmarked_only`, default `always`).
- **Reclaiming.** Dead objects are reclaimed by the sweepers (§5.5), like any other dead object.

Go's specials (finalizers, cleanups and weak handles, processed lazily at sweep) are not ported. If reference processing still dominates the FinalMark pause, the fix belongs in the shared reference processing.

### 5.15 Scavenger (phase 7)

- Free pages in `SpanSpace` chunks are released with `madvise(MADV_DONTNEED)` by a background packet in the concurrent-sweep phase.
- The retain target is Go's: `(1 + 10%) × heapGoal/lastHeapGoal × lastHeapInUse`, or 95% of the memory limit.
- A per-chunk `scavenged` bitmap mirrors `pallocData`, so that `needzero` stays correct. Scavenged pages come back zeroed.
- mmtk-core has no RSS-return mechanism today. This is new but local to `SpanSpace`.

## 6. mmtk-core API and cross-module changes

| Change | Module | Public API? | Needed from phase |
|---|---|---|---|
| `PlanSelector::ConcurrentSpan`, plan and mutator registration | plan, util/options | Yes (new variant) | 1 |
| `GCTriggerSelector::FeedbackPacer`, options `gc_percent`, `soft_mem_limit` | util | Yes (new options) | 1 |
| `SpanSpace` policy, `SpanAllocator`, page-map side metadata spec | policy, util/alloc, util/metadata | No | 2 |
| `AllocationSemantics::NoScan` (not used by mmtk-openjdk) | public API | **Yes, MEP** | 2 |
| `AllocatorInfo` for `SpanAllocator` (binding inline fast path) | util/alloc | Yes | 2 |
| Concurrent-sweep scheduling after FinalMark (reuses `Concurrent` + `concurrent_work_finished_no_pause`) | plan | No | 2 |
| Log-bit support in `SpanSpace` (SATB option only) | policy | No | 3 |
| `Collection::park_for_assist` / `unpark` (with defaults), assist worker activation | vm, scheduler | Yes | 3 |
| Plan hook for allocation failure in `Space::acquire` (stall instead of `block_for_gc`); concurrent `System.gc()` and emergency cycles | policy/space, plan, global_state | No | 3 |
| `Collection::scannable_stack_bytes()` (with default) | vm | Yes | 3 |
| `BarrierSelector::HybridBarrier`, `HybridBarrier`, `memory_manager::barrier_active_address()`, buffer offsets in the mutator | plan/barriers, memory_manager | Yes | 4 |
| `Scanning::CONCURRENT_STACK_SCAN_MODES`, `start_concurrent_stack_scan`, `scan_stacks_concurrently`, `ConcurrentScanStacks` | vm, scheduler | **Yes, MEP** | 4 |
| `memory_manager::report_stack_roots` | memory_manager | Yes | 4 |
| `Collection::flush_mutators_concurrently` (ragged flush) | vm | Yes | 4 |
| Go-style mark termination: `RaggedFlush` rounds, a FinalMark pause that can restart concurrent marking | plan, scheduler | No | 4 |
| `memory_manager::soft_references_are_strong`; concurrent scanning of finalizable objects' contents | memory_manager, util/reference_processor, util/finalizable_processor | Yes | 4 |
| Weak→strong block: the flag, `memory_manager::wait_for_weak_to_strong`, waking at the end of FinalMark | plan, memory_manager | Yes | 4 |
| `Scanning::scan_object_range` (oblets) | vm | Yes | 6 |
| Mutator-executed GC work (`memory_manager::assist`) | scheduler | **Yes, MEP** | optional |

Every new `Collection`/`Scanning` method has a default that preserves current behaviour, so existing bindings compile unchanged.

## 7. Plan and policy contracts

`ConcurrentSpan`'s `PlanConstraints`, in its default (`HybridBarrier`) form:

```rust
PlanConstraints {
    moves_objects: false,
    max_non_los_default_alloc_bytes: 32768,
    needs_log_bit: false,
    barrier: BarrierSelector::HybridBarrier,
    needs_prepare_mutator: true,          // flush span caches, barrier buffers
    may_trace_duplicate_edges: true,
    ..PlanConstraints::default()
}
```

With `concurrent_span_barrier = satb`, the plan returns a second static constant with `barrier: BarrierSelector::SATBBarrier` and `needs_log_bit: true`. The choice is made when the plan is created. Both mmtk-core and mmtk-openjdk must read the barrier from the created plan, not from a per-`PlanSelector` constant.

**Spaces:**
- `SpanSpace` for Default and NoScan;
- the common LOS for objects over 32 KiB;
- the common immortal space;
- `vm_space` if the binding needs it.

The common `nonmoving` space is not used.

**Prepare/release by pause:**

| Pause | prepare | release / on_pause_end |
|---|---|---|
| InitialMark | Finish residual sweep, flip `GcBits` epoch, prepare LOS, mutators flush span caches, scan VM roots, start the stack scan. SATB option: bulk-set unlog bits | Enable the barrier and `allocate_as_live`, pacer `startCycle`, schedule `ConcurrentScanStacks` |
| FinalMark | Flush barrier buffers. If any work: restart concurrent marking and stop here (the weak→strong block stays set). Otherwise disable the barrier | Reference processing (no tracing), `sweepgen += 2`, LOS release, pacer `endCycle`+`commit`, schedule concurrent sweep, wake stalled mutators, clear the weak→strong block and wake its waiters. SATB option: clear unlog bits |
| Full (phases 1–2, or `full_gc_fallback`) | Like InitialMark, but no barrier | Eager parallel sweep in the pause, pacer `commit` |

## 8. Implementation phases

Each phase ends with something that runs and is tested on MockVM and on OpenJDK (x86_64 and aarch64).

1. **Plan skeleton on `MarkSweepSpace`, STW only.**
   - Add `ConcurrentSpan` with `Pause::Full` only, `FeedbackPacer` as the trigger, and the eager-sweeping `MarkSweepSpace`.
   - Port the pacer and `TestGcPacer` scenarios.
   - Exit: `MMTK_PLAN=ConcurrentSpan` passes the mock tests and runs DaCapo on OpenJDK. The pacer tests pass.
2. **`SpanSpace`.**
   - Size classes, pages, spans, page map, central, `SpanAllocator`, epoch bitmaps, `sweepgen`, lazy + background sweep, `NoScan`.
   - mmtk-openjdk: emit the `SpanAllocator` inline fast path. It can trail the rest of the phase.
   - Still STW marking.
   - Exit: mock tests and a `sanity`-feature DaCapo run on OpenJDK. Sweep-termination residue is measured.
3. **Concurrent marking with the existing SATB barrier, `initial_mark` stack scan.**
   - InitialMark/FinalMark, SATB option with log bits in `SpanSpace`, allocate-black, Stage A assists (park + extra worker), allocation stalls instead of `Pause::Full`, concurrent `System.gc()` and emergency cycles, CPU limiter.
   - This gets concurrent marking running before any new barrier exists.
   - Exit: OpenJDK passes the `ConcurrentImmix` test matrix with no `Pause::Full`. Pause times and stall times are reported. A small-heap run reaches out-of-memory correctly (soft references cleared first).
4. **`HybridBarrier` and `handshake` stack scanning (Go-faithful).**
   - `HybridBarrier` in mmtk-core, and in mmtk-openjdk's interpreter, C1 and C2 on x86_64 and aarch64, including arraycopy, clone, referent loads, VM-root stores and nmethod registration.
   - mmtk-core's concurrent stack-scan API, and the handshake implementation in mmtk-openjdk.
   - Go-style mark termination: ragged flush rounds, a FinalMark that does no tracing and can restart concurrent marking, the weak→strong block, soft references traced as strong during marking, finalizable contents scanned concurrently. Until this lands (phase 3), FinalMark finishes the closure in the pause like `ConcurrentImmix`.
   - Exit: MockVM tests with mocked roots, DaCapo with `sanity` on OpenJDK, and a deep-stack stress test showing that the InitialMark pause no longer grows with stack depth. Requires the MockVM root mocking noted in §4.
5. **`watermark` stack scanning (optional, for comparison).** `StackWatermark` support in mmtk-openjdk. Exit: works with both barriers, and with `ConcurrentImmix`.
6. **Green Tea.** Span queues and inline marks, plus oblet splitting. Exit: mark-throughput comparison with phase 4 on small-object-heavy DaCapo benchmarks.
7. **Scavenger and memory-limit behaviour.** Exit: RSS tracks `soft_mem_limit` within Go's tolerance.
8. **Real assists (optional, MEP).** Only if Stage A falls short. Exit: assist time and pause-time distributions under allocation-heavy load compared with Stage A.

## 9. OpenJDK binding work

| Change in mmtk-openjdk | Why | Phase |
|---|---|---|
| Select `ConcurrentSpan`, read its constants (`max_non_los_default_alloc_bytes`, barrier) from the created plan | Plan registration; the barrier depends on an option (§7) | 1 |
| `SpanAllocator` inline fast path (`alloc_cache` ctz) in the x86_64 and aarch64 assemblers and C2 | Gap 9; needed for performance numbers | 2 |
| `park_for_assist` / `unpark` as a GC-safe block, like `block_for_gc` | Assists (§5.11) | 3 |
| `scannable_stack_bytes()`: sum of each thread's used stack | Pacer input (§5.10) | 3 |
| `MMTkHybridBarrier`: interpreter, C1 and C2 on x86_64 and aarch64; inline flag test and buffer append; arraycopy and clone; `Reference.referent` loads; `oop_store_not_in_heap` shading; shading embedded oops in `register_nmethod` while marking | `HybridBarrier` (§5.6) | 4 |
| Weak→strong block in the weak-load slow path: `Reference.get()` (interpreter, C1 and C2 intrinsics, runtime) and JNI weak global resolution call `wait_for_weak_to_strong` and reload. VM-internal weak loads only shade | Weak→strong block (§5.14) | 4 |
| `scan_stacks_concurrently` with `Handshake::execute(cl)`; the closure walks the thread's stack and calls `report_stack_roots` | `handshake` stack scanning (§5.7) | 4 |
| `flush_mutators_concurrently` with `Handshake::execute(cl)` | Ragged flush for mark termination (§5.2, §5.9) | 4 |
| When scanning a `SoftReference`, report the referent as a strong slot if `soft_references_are_strong` | Tracing-free termination pause (§5.14) | 4 |
| `MMTkStackWatermark` registered per thread in `on_thread_attach`; epoch start and `finish_processing` | `watermark` stack scanning (§5.7) | 5 |
| `scan_object_range` for `objArray` sub-ranges | Oblets (§5.9) | 6 |

The riscv port has no SATB barrier today, and would need `HybridBarrier` too, so `ConcurrentSpan` is limited to x86_64 and aarch64.

## 10. Evaluation and testing

### 10.1 Evaluation

**What `ConcurrentSpan` is compared with.**
- MMTk plans on mmtk-openjdk: `ConcurrentImmix` (the closest design: concurrent, non-moving marking with SATB), `LXR`, `Immix`, `GenImmix` and `MarkSweep`.
- OpenJDK collectors on the same JDK build: G1 (the default), Parallel, ZGC and Shenandoah.

**Harness.** DaCapo (Chopin), run through running-ng like the other MMTk evaluations, on x86_64 and aarch64, with the same JVM flags apart from the collector selection. Every MMTk plan uses the same mmtk-core and mmtk-openjdk build, so differences come from the plan, not the build.

**Heap configurations.** Go and Java size the heap in different ways, so both are reported:
- **Fixed heap**, as every other collector is usually evaluated: `-Xmx` at multiples of each benchmark's minimum heap. `ConcurrentSpan` runs with `soft_mem_limit` = `-Xmx` and `gc_percent = off`, which is Go's "memory limit only" mode (`GOGC=off` + `GOMEMLIMIT`).
- **Go's default behaviour**: `gc_percent = 100` with a generous `-Xmx`. The heap the pacer actually uses is reported next to the time, as a time/space point.

**Metrics.**

| Metric | Why |
|---|---|
| Total time and mutator time | Overall cost against the other collectors |
| STW pause p50/p99/max, split into InitialMark and FinalMark | Go's main promise, comparable with ZGC, Shenandoah and `ConcurrentImmix` |
| Ragged flush rounds per cycle, FinalMark restarts | Go-style termination behaviour |
| Weak→strong waits: count, total and max time | Cost of the weak→strong block (Go's `waitReasonGCWeakToStrongWait`) |
| Heap size actually used (and RSS) | Time/space trade-off, especially in Go's default mode |
| GC CPU time (workers + assists) | Concurrent collectors trade CPU for pauses |
| InitialMark pause vs total stack size | Shows that concurrent stack scanning works |
| Sweep residue swept in the InitialMark pause | Shows whether sweep pacing works |
| Heap at trigger and at mark end vs goal | Pacer fidelity |
| Assist time fraction, mutator park time, extra workers activated | Assist behaviour |
| Allocation stalls: count, total and max time | With no STW full GC, this is where a too-small heap shows up. Comparable with ZGC's allocation stalls |
| Mark throughput (scan bytes per worker-second) | Green Tea benefit |
| Fragmentation (reserved / live) | Size-class fidelity |

**Ablations.** Each main Go feature has a switch, so its effect can be measured on its own:
- `concurrent_span_barrier`: `HybridBarrier` vs SATB. With SATB, `ConcurrentSpan` and `ConcurrentImmix` share a barrier, so the difference comes from the heap and the rest of the design;
- `stack_scan`: `initial_mark`, `handshake` and `watermark`;
- assists on or off;
- Green Tea on or off;
- `FeedbackPacer` vs other triggers for `ConcurrentSpan`, and `FeedbackPacer` on `ConcurrentImmix`. Together these separate the effect of Go's heap from the effect of Go's pacer.

**Threats to validity.** The deviations in §11 can move results in either direction. The ones most likely to matter are assists, Green Tea without SIMD, noscan objects, and the barrier's extra cases on OpenJDK (VM-root stores and nmethods). Results should be reported with that table. Pause times must also be reported together with stall times: a collector with no full GC moves the cost of a small heap from pauses into stalls.

### 10.2 Testing

- **Pacer:** port `TestGcPacer` scenarios as Rust unit tests. Results must match Go's within the same tolerances the Go test uses.
- **Size classes:** a generated-table test that checks every class's waste bound and `div_magic` exactness, as `mksizeclasses.go:140` proves.
- **MockVM:**
  - extend `mock_vm` so roots can be mocked, including the concurrent stack-scan hooks;
  - add `mock_test_concurrent_span_*` for alloc/sweep/epoch correctness, `NoScan` spans, allocate-black, a `HybridBarrier` buffer flush (both old and new values), the SATB option, object-index lookup at every offset of every class, assist parking and release, and a concurrent mutator + marker stress test;
  - CI picks up `MMTK_PLAN=all` automatically.
- **Sanity GC:** run every phase with the `sanity` feature on OpenJDK DaCapo, and with `MMTK_STRESS_FACTOR`.
- **OpenJDK:** DaCapo on x86_64 and aarch64 in every phase, plus a deep-recursion stress test for phase 4 and an allocation-heavy test for assists.
- **Per phase:** the evaluation metrics above that apply to the phase.

## 11. Deliberate deviations from Go

| Go | `ConcurrentSpan` | Reason | Effect on the evaluation |
|---|---|---|---|
| Hybrid Yuasa/Dijkstra barrier, flag-gated per write | Same (`HybridBarrier`), plus shading of VM-root stores and new nmethods' oops | Java has VM roots outside the heap and JIT-compiled code roots | Slightly more barrier work than Go in rare paths. The SATB option is a deliberate, separate configuration |
| Workers suspend each goroutine and scan its stack | Each thread scans its own stack in a non-safepoint handshake; the VM thread scans blocked threads | HotSpot's single-thread synchronous handshake needs a Java thread as the caller | Stack-scan work runs on the mutator's CPU, not a GC worker's. Blocked threads are scanned serially |
| Globals scanned concurrently (they are barriered) | VM roots scanned in the InitialMark pause | OpenJDK's VM roots are not ordinary heap memory. Later option (§12) | InitialMark pause includes VM roots, which Go's does not |
| No STW full GC; the heap grows past the soft limit when needed | No STW full GC; at `-Xmx`, allocating threads stall until the concurrent cycle frees memory, then out-of-memory after an emergency cycle | Java has a hard heap limit | Small heaps show stalls instead of heap growth. Report stall time with pauses |
| Weak→strong conversions block during mark termination | Same for `Reference.get()` and JNI weak globals. VM-internal weak handles shade instead of blocking | Blocking inside VM code while holding VM locks could deadlock | VM-internal weak loads can add work during termination, costing extra ragged rounds |
| Mark termination does no tracing and restarts mark if work is found | Same, from phase 4. FinalMark also does Java reference processing (lists only) and VM weak roots | Java semantics, through MMTk's mechanism | FinalMark is somewhat longer than Go's mark termination, by the reference and weak-root processing |
| Fractional and idle mark workers | Fixed `concurrent_threads` OS-thread workers, plus assist workers | MMTk workers are not scheduled on Ps | Idle CPU is used by the OS, not by idle mark workers. Mark may finish later on idle machines |
| Assists run mark work on the allocating goroutine | The mutator parks and an extra worker runs mark work (Stage A) | Mutators cannot run GC packets without an MEP-level scheduler change | Adds a park and wake per assist, and loses the mutator's cache locality |
| Pointer-free objects in noscan spans, never scanned | On OpenJDK every object is in a scan span | `NoScan` is not used by mmtk-openjdk | Marking does more work than Go for primitive arrays: a queue push and an empty `scan_object` per object |
| Specials (finalizers, cleanups, weak handles) processed lazily at sweep | MMTk reference and finalizer processing in FinalMark | Java semantics, and MMTk's own mechanism | The same for every MMTk plan compared. FinalMark includes reference processing |
| Green Tea scans spans through span-tail pointer bitmaps, with SIMD | Green Tea span queues, but `scan_object` per object (GT-2) | OpenJDK describes reference fields per class, not per span | Green Tea's benefit is understated: locality, but no SIMD |
| Per-P span rings, stealable through `spanSPMC` chains | A span ring per tracing packet; spills become MMTk work packets (GT-3) | MMTk's scheduler owns load balancing | Spans are shared at packet granularity; stealing is coarser than Go's |
| Mark bits in `gcBits` arenas, swapped with the alloc bits at sweep; Green Tea inline marks | Fixed per-span mark and alloc bitmaps in the span descriptor; sweep copies marks into the alloc bits and clears the marks (deviation SP1, decision D31). Green Tea's scan bits and ownership byte are in the descriptor too (GT-1, D70) | Simpler, same invariant (marks cleared per span by sweeping) | None on GC behaviour; slightly more descriptor memory |
| Heap grows by 64 MiB arenas; radix-tree page allocator over 4 MiB chunks; free pages kept and scavenged | 128 KiB page regions from MMTk's free-list page resource; linear first fit over regions; empty regions returned to MMTk at once (SP2, D30) | MMTk owns the address space and heap accounting; MockVM tests use 1 MiB heaps | Some tail fragmentation per region; RSS tracks live data more closely than Go without its scavenger |
| 64 MiB `heapArena` with per-arena page maps | Page-map side metadata over MMTk 4 MiB chunks | MMTk owns the address space layout | Negligible |
| Randomised heap base, fixed arena hints | MMTk VMMap placement | MMTk owns the layout | Negligible |

## 12. Risks and open questions

**Long-term risks to mmtk-core:**
- Concurrent stack scanning is a scheduler and binding-API change that every future concurrent plan inherits. It must be designed as a general mechanism (`ConcurrentImmix` should be able to use it) and needs an MEP before phase 4.
- `HybridBarrier` adds a barrier kind whose correctness depends on every root store being covered (§5.6). A missed path loses live objects silently, so it needs targeted tests and `sanity` runs.
- `AllocationSemantics::NoScan` is a public API addition. Plans that do not care must map it to `Default`, and that must stay cheap.

**Risks on the OpenJDK side:**
- `HybridBarrier` is the largest piece of binding work: interpreter, C1 and C2 on two architectures, plus arraycopy, clone, referent loads, native root stores and nmethods.
- `Handshake::execute(cl)` goes through the VM thread, which scans blocked threads one after another. With many blocked threads, stack scanning can lag behind marking.
- Handshakes and `StackWatermark` are HotSpot-internal APIs. mmtk-openjdk will have to follow them across JDK updates, as ZGC and Shenandoah do in-tree.
- Until the `SpanAllocator` inline fast path lands, allocation-heavy benchmarks will measure the slow path, not the collector.

**Correctness hotspots:**
- The `sweepgen` protocol between mutator sweeping, background sweeping and InitialMark.
- Barrier enable/disable visibility at the pause boundaries.
- Allocate-black with the per-span bitmaps (and the unlog bits, with the SATB option).
- Completeness of `HybridBarrier` on OpenJDK: every store that can hide a reference from an unscanned stack.
- A thread scanning its own stack (handshake closure or watermark trap) reporting into its own barrier buffer.
- Assist parking and the release at mark end.
- Mark termination: a ragged round that wrongly reports no work only costs a restart, but a FinalMark check that misses a buffer would lose objects.
- Weak→strong block: every Java-visible weak→strong path must block or shade; a thread must never park while holding a lock the GC needs; and waiters must be woken in the same pause that clears the flag.
- Soft references traced as strong, and finalizable contents scanned concurrently: both change when MMTk's reference and finalizer processing traces.
- Allocation stalls: waking stalled mutators only after FinalMark, raising the worker cap while anyone is stalled, and counting completed cycles correctly for out-of-memory.

**Open questions:**
1. Page resource for 4 MiB `SpanSpace` chunks: `BlockPageResource` with a chunk-sized block, or an aligned `FreeListPageResource`? *Decided (D30): `FreeListPageResource`, 128 KiB page regions.*
2. Should `GcBits` arenas use MMTk's side-metadata mmapper, or plain `mmap`? This affects memory accounting. *Decided (D31): no arenas; fixed bitmaps in the span descriptor.*
3. nmethods registered during concurrent marking: with `HybridBarrier`, registration shades their embedded oops (§5.6). With the SATB option, are they always reachable from the snapshot or newly allocated? `ConcurrentImmix` has the same question today.
4. Can VM roots (OopStorage, class loader data) and VM weak roots be processed concurrently, as Shenandoah does, to shrink the InitialMark and FinalMark pauses further?
5. Should we go beyond Go and remove the FinalMark pause entirely (§5.2)? It needs thread-local barrier flags flipped by handshake, an epoch change without stopping threads, and concurrent reference processing with a `Reference.get()` barrier.
6. Should mmtk-openjdk add a binding-owned Java thread so GC workers can request single-thread synchronous handshakes? That would let blocked threads be scanned in parallel, closer to Go's workers calling `suspendG`.
7. Is Stage A close enough to Go for the pacer simulations to hold? The pacer's `u` includes assist time, so park time and extra-worker time must be accounted as assist time.
8. Should `FeedbackPacer` be usable with other concurrent plans? It can serve `ConcurrentImmix` too, given the same inputs.

## 13. Implementation status (2026-10-12)

What exists on `mmtk-core-go-gc` and mmtk-openjdk `concurrent-span` after the first session. Details and every simplification are in the [decisions](concurrent-span-decisions.md) and [deviations](concurrent-span-go-deviations.md) documents.

| Phase | Item | Status |
|---|---|---|
| 1 | `ConcurrentSpan` plan, `FeedbackPacer` (all 18 `TestGcPacer` scenarios pass), CPU limiter | Done. The pacer is driven from the plan (D9), not a `GCTriggerSelector` |
| 2 | `SpanSpace`: Go size classes (generated), spans, page map, central, `sweepgen`, lazy + background sweeping, allocate-black | Done, with simplifications SP1–SP12 (128 KiB page regions, per-span bitmaps, mutex central lists) |
| 2 | `AllocationSemantics::NoScan` | Deferred (D33) |
| 2 | OpenJDK inline allocation fast path (`nextFreeFast`) | Done on x86_64 interpreter, C1 and C2 (D80–D87, FP-1..5); bails to the slow path while allocating black. aarch64 not done |
| 3 | Concurrent marking, InitialMark/FinalMark, SATB option | Done |
| 3 | Allocation stalls instead of a STW full GC | Done (D10, D16) |
| 3 | Assists, Stage A (park + extra worker), CPU limiter gating | Done (D17, PLAN-2, PLAN-3) |
| 3 | `System.gc()` as a concurrent cycle blocking only the caller | Done (D11) |
| 4 | `HybridBarrier`: core, and mmtk-openjdk interpreter/C1/C2 on x86_64, arraycopy, clone, weak loads, VM-root stores, nmethods | Done on x86_64; aarch64 not implemented (D58) |
| 4 | Concurrent stack scanning with handshakes; ragged flush; Go-style mark termination with restarts | Done |
| 4 | Soft references strong while marking; finalizable contents scanned concurrently | Done (D15) |
| 4 | Weak→strong block during mark termination | Not done (D18) |
| 5 | `StackWatermark` stack scanning | Not started |
| 6 | Green Tea span scanning | Done (D70–D75, GT-1..7); no oblets |
| 7 | Scavenger, memory-limit RSS behaviour | Not started |
| 8 | Real assists (MEP) | Not started |

**Testing.** 34 MockVM test files run with `ConcurrentSpan`, 16 of them specific to it (including real concurrent cycles, stalls, assists, user GCs during marking, Green Tea, a multi-threaded stress test), all passing with Green Tea on and off and with the SATB option. The other plans' mock tests still pass. On OpenJDK (fastdebug, x86_64), `ConcurrentSpan` runs a barrier stress test in the interpreter, C1 and C2, and DaCapo fop, luindex, lusearch (`-s small`) and h2 (`-s small`) pass with C1 only, C2 only and (fop, h2) tiered, with and without `MMTK_HYBRID_BARRIER_DEBUG_ALWAYS_ON`. Default-size lusearch is slow rather than stuck: with `gc_percent=100` its small live heap gives a heap goal of about 27 MB, so thousands of cycles run (as Go would with `GOGC=100`). Fastdebug `-Xint` is too slow for the DaCapo harness. After the barrier began dropping already-marked objects at flush time (as `wbBufFlush1` does), FinalMark restarts on fop fell from 53 in 66 cycles to 0–1.

**Evaluation kit.** `~/Code/concurrent-span-eval` (running-ng configs) compares `ConcurrentSpan` (both heap modes, and the ablations of §10.1) with `ConcurrentImmix`, G1, ZGC and Shenandoah. It needs a release build (with the inline allocation fast path, now merged) before its numbers mean anything.

**First quick evaluation (2026-10-12).** Release build, `concurrent-span-quick.yml`, 3× G1's minimum heap, one invocation, timed iteration (ms):

| Benchmark (heap) | G1 | ZGC | Shenandoah | ConcurrentImmix | ConcurrentSpan, memory limit only | ConcurrentSpan, `GOGC=100` |
|---|---|---|---|---|---|---|
| fop (39 MB) | 1860 | 4004 | 2242 | 1817 | 11384 | 16308 |
| luindex (87 MB) | 7603 | 7363 | 8138 | 7114 | 7346 | 9687 |
| lusearch (63 MB) | 18905 | 32157 | 34038 | 23921 | 29071 | 48185 |

What this run found and fixed on the way (D24, D25, SP2): page allocation scanned every region (35% of lusearch's CPU near a 1 GiB limit); nothing fed the pacer's memory-limit goal, so the hard limit started every cycle with no runway; unswept garbage counted against the hard limit, so a cycle could start straight after the previous one and sweep the heap in its `InitialMark` pause. fop at 60 MB went from 13 s to 2.4 s (ConcurrentImmix: 1.8 s).

Remaining observations:
- fop at 39 MB is **not** a capacity limit (an earlier version of this note said it was). Marked bytes grow by about 4-5 MB per iteration with concurrent marking (fop, 200 MB heap, 10 iterations: 16 → 52 MB; at 39 MB the heap grows past `-Xmx`, to 15215 of 9984 pages). With `concurrent_span_disable_concurrent_marking` it stays at 12 MB, fop's live size. The growth does not depend on reference processing (also with `MMTK_NO_REFERENCE_TYPES`; stop-the-world with it stays at 12 MB), stack scanning (`scan_stacks_in_pause`), the barrier (hybrid or SATB), assists, Green Tea or `System.gc()`, and is per iteration rather than per cycle. ConcurrentImmix stays flat over 25 iterations at 39 MB. `bytes_marked` matches the mark bits (debug check), so the objects are traced: something keeps dead objects reachable in concurrent cycles. Open. Separately, the heap growing past `-Xmx` means some path takes pages past the hard limit.
- `GOGC=100` is expensive at these heap sizes because the live heaps are small (Go's policy, not a bug); compare at equal memory with the memory-limit mode.
- Pauses are short: on lusearch the median pause is 1.0 ms (ConcurrentImmix: 6.9 ms) and the total pause time 2.3 s (ConcurrentImmix: 5.6 s). The time lost is mutator time (26.8 s against 18.3 s outside pauses). A profile shows the mutators mostly in JIT-compiled code, with the allocation slow paths at about 4% and the barrier under 0.5%, so the likely cause is locality: size-segregated free-list allocation scatters objects that Immix allocates contiguously. Whole-process user-mode counters (`perf stat`, lusearch, 63 MB, GC threads included) agree: against ConcurrentImmix, ConcurrentSpan has 15% more cycles, 11% more instructions, 7% more L1 data misses, 58% more LLC load misses and 66% more dTLB load misses. Separating mutator from GC counts is the next step.
- During marking every allocation still takes the slow path (allocate-black in the binding's fast path is not implemented, FP-4).

## Appendix A. Go → mmtk-core → OpenJDK concept map

| Go | mmtk-core (`ConcurrentSpan`) | OpenJDK |
|---|---|---|
| P (`mcache`, `wbBuf`, `gcWork`) | `Mutator` (`SpanAllocator`, `HybridBarrier` buffer) + `GCWorker` | Java thread |
| G stack | Scanned by `ConcurrentScanStacks` | Java thread stack, scanned in a handshake |
| `suspendG` + `scanstack` | `scan_stacks_concurrently`, `report_stack_roots` | `Handshake::execute(cl)` |
| `writeBarrier.enabled` | `barrier_active_address()` | Tested inline by the interpreter, C1 and C2 |
| `mheap` / `pageAlloc` | `SpanSpace` page allocator over 4 MiB chunks | |
| `mspan` | `Span` descriptor (off-heap table) | |
| `heapArena.spans` | Page-map side metadata | |
| `mcentral` | `central.rs` per-class span sets | |
| `gcBits` / `gcBitsArena` | `GcBits` epoch arenas | |
| `sweepgen` | `SpanSpace::sweepgen` + `Span::sweepgen` | |
| `gcStart` STW | `Pause::InitialMark` | Safepoint |
| `gcMarkDone` (ragged barrier) | `RaggedFlush` rounds | `Handshake::execute(cl)` |
| `gcMarkTermination` STW | `Pause::FinalMark`, no tracing, can restart concurrent marking | Safepoint |
| `gcScanFinalizer` | Concurrent scan of finalizable objects' contents | |
| `work.strongFromWeak.block`, `gcParkStrongFromWeak` | Weak→strong block flag, `wait_for_weak_to_strong` | `Reference.get()`, JNI weak globals |
| `gcBgMarkWorker` | GC workers in `WorkBucketStage::Concurrent` | |
| `gcDrain` / `scanobject` | `SpanTraceObjects` | `scan_object` via oop maps |
| `wbBufFlush1` | `ProcessHybridBuf` packet | `MMTkHybridBarrier` in interpreter, C1, C2 |
| `bgsweep` | `SweepSpans` packets in `Concurrent` after FinalMark | |
| `gcControllerState` | `FeedbackPacer` | `-Xmx` as the hard limit |
| `gcAssistAlloc` | `assist.rs`: park + extra worker | `park_for_assist` as a GC-safe block |
| specials (finalizer, cleanup, weak) | `ReferenceProcessors` and `FinalizableProcessor`, in FinalMark | `java.lang.ref`, finalization |
| `findObject` | `SpanSpace` object-index lookup | |
| Large span (class 0) | Common LOS | |

## Appendix B. Key source references

**Go (`go/src/`):**
- `runtime/mgc.go`: cycle, STW points, termination.
- `runtime/mgcmark.go`: roots, assists, drain, allocate-black.
- `runtime/mgcmark_greenteagc.go`: span scanning.
- `runtime/mgcpacer.go`: pacer. `runtime/mgcpacer_test.go`: pacer simulations.
- `runtime/mgclimit.go`: CPU limiter.
- `runtime/mgcsweep.go`: sweeping.
- `runtime/mbarrier.go`, `runtime/mwbbuf.go`: barrier.
- `runtime/mheap.go`, `runtime/mcentral.go`, `runtime/mcache.go`, `runtime/mpagealloc.go`: heap.
- `runtime/mbitmap.go`: pointer metadata and `findObject`.
- `internal/runtime/gc/sizeclasses.go`, `runtime/_mkmalloc/mksizeclasses.go`: size classes.
- `cmd/compile/internal/ssacompile/writebarrier.go`: compiler barrier.

**mmtk-core:**
- `src/plan/concurrent/`: pause machine, SATB, concurrent tracing.
- `src/plan/barriers.rs`: barrier API.
- `src/policy/marksweepspace/native_ms/`, `src/util/alloc/free_list_allocator.rs`: current size-segregated heap.
- `src/util/heap/gc_trigger.rs`: trigger policies and `poll`.
- `src/scheduler/scheduler.rs`, `src/scheduler/work_bucket.rs`: concurrent buckets and the drained hook.
- `src/vm/scanning.rs`: `process_weak_refs`.
- `src/util/test_util/mock_vm/`: MockVM.
- `src/scheduler/worker_monitor.rs`: active worker count.

**OpenJDK (`src/hotspot/share/`, jdk-21.0.9):**
- `runtime/stackWatermark.hpp`, `runtime/stackWatermarkSet.hpp`: concurrent stack processing.
- `gc/shenandoah/shenandoahStackWatermark.cpp`, `gc/shenandoah/shenandoahRootProcessor.cpp`: concurrent marking of thread stacks with an SATB barrier.
- `gc/z/zStackWatermark.cpp`: the same mechanism in ZGC.
- `runtime/handshake.hpp`: thread-local handshakes.

**mmtk-openjdk (`jdk-21`):**
- `openjdk/share/barriers/mmtkSATBBarrier.cpp`, `openjdk/cpu/{x86,aarch64}/mmtkSATBBarrierSetAssembler_*`: SATB barrier.
- `openjdk/share/mmtkUpcalls.cpp`: `mmtk_scan_roots_in_mutator_thread`, `MMTkBookKeepingCodeBlobClosure`.
- `openjdk/share/mmtkHeap.cpp`, `mmtk/src/gc_work.rs`: code-cache root set.
- `openjdk/cpu/x86/mmtkBarrierSetAssembler_x86.cpp`: inline allocation fast path.

**Design documents:**
- Go proposal 17503 (eliminate stack rescan).
- JEP 376 (ZGC: concurrent thread-stack processing), which introduced `StackWatermark`.
- Go proposal 44167 (pacer redesign).
