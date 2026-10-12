# ConcurrentSpan: deviations from Go's code

Every place where our implementation differs from the Go code it ports. The port exists to evaluate Go's collector, so every difference has to be visible when results are reported. The rules are in [`concurrent-span-engineering.md`](concurrent-span-engineering.md) §3.

**Go reference:** `golang/go` master `2c3c1caad2` (2026-10-10). Paths are relative to `go/src/`.

**Scope.** Design-level deviations (barrier coverage, stack scanning, allocation stalls, reference processing, and so on) are summarised in [`go-gc-port.md`](go-gc-port.md) §11. This file records the code-level ones: places where a ported function, constant, table or state machine does something different from Go's code. When a code-level deviation changes the design, update §11 as well.

**Not deviations:** using MMTk's integration points (work packets, `Space`, options, side metadata), Rust idioms, and renaming to avoid "Go" in identifiers. Those are expected (engineering §3).

**Effect on the evaluation:** say which metrics could move and in which direction, or "none expected".

| ID | Go code | What Go does | What ConcurrentSpan does | Why | Effect on the evaluation | Our code | Design ref |
|---|---|---|---|---|---|---|---|
| PACER-1 | `runtime/mgcpacer.go:59-60` `defaultHeapMinimum` | 512 KiB with the `HeapMinimum512KiB` experiment, else 4 MiB | Always 4 MiB (`DEFAULT_HEAP_MINIMUM`) | The experiment is off by default (decision D20) | None expected for default Go | `src/plan/concurrent/span/pacer.rs` | §5.10 |
| PACER-2 | `runtime/mgcpacer.go:1045-1061` `memoryLimitHeapGoal` | Reads `heapFree`, `totalAlloc - totalFree` and `mappedReady` from runtime counters; spins until they are consistent (deadlocks on a persistent accounting error) | The caller passes `mapped_ready`, `heap_free` and `heap_alloc` through `set_memory_stats`; retries 100 times, then treats non-heap memory as 0 | MMTk has no `totalAlloc`/`totalFree` counters; a hang is worse than a slightly wrong goal | Memory-limit goal depends on how the integrator measures these stats (design §5.10 table) | `pacer.rs` | §5.10 |
| PACER-3 | `runtime/mgcpacer.go:421-431, 433-451, 695-908, 1427-1505` | STW debug mode (`gcstoptheworld`), per-P assist/fractional time reset, idle mark worker limits, `enlistWorker`, `assignWaitingGCWorker`, `findRunnableGCWorker`, idle-worker counting (`TestIdleMarkWorkerCount`) | Not ported. `start_cycle` still computes `dedicated_mark_workers_needed` and `fractional_utilization_goal`; `periodic` is accepted but unused; `mark_worker_stop` keeps the time accounting | Go scheduler scaffolding with no MMTk equivalent (design §5.9: MMTk workers are OS threads); skipped on the project lead's guidance | Idle-mark time is 0 unless reported, so cons/mark counts only background + assist CPU (as Go does when no CPU is idle) | `pacer.rs` | §5.9, §11 |
| PACER-4 | `runtime/mgcpacer.go:974-984` `addScannableStack` | Batches per P with `maxStackScanSlack` | `add_scannable_stack` adds directly (Go's `pp == nil` path); callers may batch | No Ps | None expected | `pacer.rs` | §5.10 |
| PACER-5 | `runtime/mgcpacer.go:1516-1537` `gcControllerCommit` | Also calls `gcPaceSweeper` and `gcPaceScavenger`, and emits a trace event | `controller_commit` does commit + revise + trigger only; the caller paces sweeping | Sweeping and the scavenger live elsewhere | None expected, if the caller paces the sweeper | `pacer.rs` | §5.5 |
| PACER-6 | `runtime/mgcpacer.go:458-467, 674-683` | `gcpacertrace` debug output | Not ported (no logging dependency in the pure module) | Keep `pacer.rs` std-only | None; the integrator can log the getters | `pacer.rs` | — |
| PACER-7 | All of `gcControllerState` | Mix of sequentially consistent atomics and plain fields written with the world stopped | Every field is an atomic with `Relaxed` ordering; all methods take `&self`, and STW-only methods document that the caller provides exclusion | Lets the pacer sit in a shared `GCTriggerPolicy` (decision D21) | None expected: Go documents these races as benign | `pacer.rs` | — |
| PACER-8 | Float-to-integer conversions in `revise`, `commit` | Out-of-range conversions are architecture-dependent (amd64 gives `MinInt64`) | Rust `as` saturates, and integer arithmetic wraps as in Go | Rust semantics | Only on degenerate inputs, e.g. the first cycle with no globals (`scanWorkExpected == 0`); matches Go on arm64 | `pacer.rs` | — |
| LIMIT-1 | `runtime/mgclimit.go:200-226, 340-481` | `updateLocked` consumes in-flight per-P limiter events (`limiterEvent`), and idle events also update `sched.idleTime` | Not ported: assist and idle time are only counted when reported through `add_assist_time` / `add_idle_time` (Go's test-mode behaviour) | No Ps; skipped on the project lead's guidance | Long-running assists or stalls are counted late, so the limiter may turn on a little later | `src/plan/concurrent/span/cpu_limiter.rs` | §5.11 |
| LIMIT-2 | `runtime/mgclimit.go:281, 333` `lastEnabledCycle` | Uses `memstats.numgc + 1` | Uses a cycle count set by the caller (`set_num_gc`) | No `memstats` | Metric only | `cpu_limiter.rs` | — |
| TEST-1 | `runtime/mgcpacer_test.go:930-935` `random` | `math/rand` with a fixed seed | SplitMix64 with the same seed | Go's generator (and its 607-entry seed table) is not in the reference checkout (decision D22) | Test only: `HighGOGC`, `JitterAlloc` and `HeavyJitterAlloc` see different jitter of the same amplitude; all pass Go's tolerances | `src/plan/concurrent/span/pacer/tests.rs` | §10.2 |

## Details

Longer notes for entries that need them. Use the entry's ID as the heading.
