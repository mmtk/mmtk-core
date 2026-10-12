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
| D20 | 2026-10-12 | Which `defaultHeapMinimum` (Go has 4 MiB, or 512 KiB under the `HeapMinimum512KiB` experiment)? | 4 MiB, Go's default | Yes | for review |
| D21 | 2026-10-12 | How is the pacer shared between mutators, workers and pauses? | All fields atomic (`Relaxed`), all methods `&self`; STW-only methods document their precondition | Yes | for review |
| D22 | 2026-10-12 | Go's `math/rand` source is not in the reference checkout. How are `TestGcPacer`'s random streams ported? | SplitMix64 with the same seeds and amplitudes | Yes | for review |
| D23 | 2026-10-12 | The pacer agent's worktree branch was created from `master`, not `mmtk-core-go-gc`, and a hard reset was refused | Work on a new branch `concurrent-span-pacer` created from `mmtk-core-go-gc` in the same worktree; the original worktree branch is left untouched | Yes | for review |

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
