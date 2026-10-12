# ConcurrentSpan: engineering practices

How we build the `ConcurrentSpan` port (the design is in [`go-gc-port.md`](go-gc-port.md)). These rules apply to everyone working on the port, including coding agents.

## Documents

| Document | What it holds | Updated when |
|---|---|---|
| [`go-gc-port.md`](go-gc-port.md) | The design | The design changes |
| This file | How we work | The practices change |
| [`concurrent-span-upstream-changes.md`](concurrent-span-upstream-changes.md) | Bug fixes and improvements to existing mmtk-core code, each in its own commit | A separable commit is made, rebased or upstreamed |
| [`concurrent-span-go-deviations.md`](concurrent-span-go-deviations.md) | Every place where our code differs from Go's code | We deviate from Go while porting |
| [`concurrent-span-decisions.md`](concurrent-span-decisions.md) | Decisions made without asking, for review | A question comes up that would otherwise need an answer from the project lead |

## 1. Test on MockVM first

Most of the early work cannot run on OpenJDK yet: the binding work comes later, and some phases change only mmtk-core. MockVM can run real GCs (since #1408), so we test there first.

- **Pure logic is tested without MockVM.** The pacer, size classes, `div_magic`, span arithmetic and the `sweepgen` state machine are plain `#[cfg(test)]` unit tests. The pacer's tests are ports of Go's `TestGcPacer` scenarios (design §10.2).
- **Anything that needs an MMTk instance is a MockVM test.** Allocation, sweeping, the epoch bitmaps, allocate-black, barrier buffer flushes, assists, stalls, mark termination and stack scanning.
  - Name the file `src/vm/tests/mock_tests/mock_test_concurrent_span_<what>.rs`. A test file name must not be a prefix of another one, because CI runs `cargo test -- <file name>` per file.
  - Each file runs in its own process and creates at most one MMTk instance. Register it in `src/vm/tests/mock_tests/mod.rs`.
  - Put `// GITHUB-CI: MMTK_PLAN=ConcurrentSpan` (and `// GITHUB-CI: FEATURES=...` if needed) at the top, so `ci-test.sh` runs it.
  - Run one test with:
    ```
    MMTK_PLAN=ConcurrentSpan cargo test --features mock_test -- mock_test_concurrent_span_<what>
    ```
- **Existing mock tests must pass with `ConcurrentSpan`.** Tests marked `MMTK_PLAN=all` pick up every plan in `PlanSelector`, so they run with `ConcurrentSpan` as soon as it is registered.
- **Gaps in MockVM that we fill ourselves**, as separable commits (§2):
  - Roots cannot be mocked: `scan_roots_in_mutator_thread` and `scan_vm_specific_roots` only log a warning (`src/util/test_util/mock_vm/vm.rs`). Concurrent stack scanning (phase 4) needs this.
  - Per-test VM configurations (`define_mock_vm!`) exist on the `configurable-mock-vm` branch, not on master. See decision D2.
- **Then OpenJDK.** Once binding work exists for a phase, run the `sanity`-feature build and a DaCapo subset on OpenJDK (design §10.2). Until then, a phase is not finished, but it can be committed if MockVM tests pass.

**Before every commit** (with the memory cap from §6):
```
CAP="systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=0"
cargo fmt
$CAP cargo clippy -j 8 --features mock_test --tests --benches
$CAP cargo test -j 8                         # unit tests
# mock tests that the change can affect, with the plans they need, each with
# MMTK_GC_TRIGGER=FixedHeapSize:<bytes> and under $CAP
```

## 2. Separable commits for fixes and improvements

When we fix a bug or improve existing mmtk-core code (code that exists without `ConcurrentSpan`), the change goes in **its own commit**, so it can be cherry-picked and sent upstream on its own.

A separable commit:
- contains only that change. No `ConcurrentSpan` code, and no unrelated formatting;
- depends only on code that exists upstream. It must apply to `master` with `git cherry-pick`;
- builds and passes the existing tests on its own;
- has a subject written for upstream (it should make sense to someone who has never heard of `ConcurrentSpan`).

Order:
- Make separable commits **before** the port commits that rely on them, so the branch can be split cleanly.
- Record the commit in [`concurrent-span-upstream-changes.md`](concurrent-span-upstream-changes.md) in a **separate, docs-only commit**. Never put the doc update in the separable commit itself, or cherry-picking it would drag the doc along.
- Hashes change when the branch is rebased. The upstream-changes doc records the subject as the stable key, and the hash at the time of writing.

Everything else is a **port commit**: `ConcurrentSpan`-specific code, tests and docs. Each port commit leaves the branch building with its tests passing.

## 3. Fidelity to Go vs. fitting into MMTk

The port exists to evaluate Go's collector (design §1), so results must reflect Go's design. But the code lives in mmtk-core and must read like mmtk-core.

**Follow Go** for:
- algorithms, state machines and their ordering (cycle phases, `sweepgen`, termination, pacer arithmetic, assists);
- constants and tables (size classes, buffer sizes, pacer constants, `gcCreditSlack`). Generated tables stay generated from Go's generators;
- the structure of ported functions. Keep a ported function close to the Go original, so the two can be compared side by side. Cite the Go source in a comment: `// Go: runtime/mgcpacer.go:403-419 (2c3c1caad2)`.

**Follow MMTk** for:
- every integration point: `Plan`, `Space`, `SFT`, work packets and buckets, the scheduler, options, side metadata, page resources, `VMBinding` traits;
- Rust idioms, error handling and naming. No "Go" in any identifier or option (design §2);
- anything the VM decides (object layout, roots, weak references), through MMTk's existing mechanisms.

**Record every deviation from Go's code** in [`concurrent-span-go-deviations.md`](concurrent-span-go-deviations.md): what Go does, what we do, why, and how it could affect the evaluation. Design-level deviations are summarised in design §11. If a code-level deviation changes the design, update §11 too.

Files translated from Go source or generated from Go's generators carry Go's BSD-3-Clause notice and are listed in mmtk-core's `COPYRIGHT` (design §2).

## 4. Commits, not pushes

- Commit on the port branch (`mmtk-core-go-gc` in this worktree) as work progresses.
- **Never push**, open pull requests, or publish anything. The project lead decides when and where commits go.
- Do not rewrite history the project lead may have seen (no force updates of other branches, no rebasing their commits) without recording why in the decisions doc.

## 5. Working autonomously

Work proceeds without stopping to ask. When a question comes up that the project lead would normally answer:

1. Pick the option that best fits the design doc and the goal of a faithful evaluation. When unsure, prefer the option that is easier to reverse.
2. Record it in [`concurrent-span-decisions.md`](concurrent-span-decisions.md): the question, the options, the choice, why, and how to undo it.
3. Carry on.

Things that are never done autonomously, even with a decision record: pushing, publishing, deleting someone else's work, or changing anything outside this worktree. Those are written up as an open item in the decisions doc instead.

## 6. Bounded memory

Never run anything whose memory use is unbounded. The development machine (31 GB RAM, 24 cores) is shared with other work, and a runaway process can take the whole machine down.

**Cap every process's resident memory with a cgroup:**
```
systemd-run --user --scope -q -p MemoryMax=<limit> -p MemorySwapMax=0 <command>
```
- The limit covers the command and every process it starts. If they go over, the kernel kills them (exit status 137). This has been checked on the development machine.
- `MemorySwapMax=0` stops the process from escaping the cap by swapping.
- Do not use `ulimit -v`: MMTk reserves a large virtual address range up front, so it fails even when real memory use is small. Do not rely on `prlimit --rss`: Linux does not enforce it.

**Default limits** (decision D5):

| What | Cap | Also |
|---|---|---|
| `cargo build`, `cargo clippy`, `cargo test` | `MemoryMax=8G` | `-j 8` to limit parallel compilers and linkers |
| A MockVM test | `MemoryMax=4G` | `MMTK_GC_TRIGGER=FixedHeapSize:<bytes>`, normally 1 GB or less |
| An OpenJDK run | `-Xmx` + 2 GB | Always set `-Xmx` (and `-Xms`). The extra 2 GB covers metaspace, code cache, thread stacks and MMTk side metadata |

**Always bound the heap.** Never run MMTk with an unbounded trigger. MockVM tests use `FixedHeapSize`. `FeedbackPacer` runs always have a maximum heap size (and `soft_mem_limit`), even when measuring Go's default `gc_percent` behaviour.

**Keep the total small.** Processes running at the same time must have caps that add up to at most 16 GB, half the machine. In practice, run one OpenJDK benchmark at a time.

**Bound time too.** Run long jobs under `timeout`, so a hang does not hold memory forever.

**Bound the tests themselves.** A stress test or allocation loop must have a fixed number of iterations or a fixed total allocation, not "until something happens".

**Measure.** For benchmark and stress runs, wrap the command in `/usr/bin/time -v` and record "Maximum resident set size" next to the results.
