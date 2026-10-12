# ConcurrentSpan: deviations from Go's code

Every place where our implementation differs from the Go code it ports. The port exists to evaluate Go's collector, so every difference has to be visible when results are reported. The rules are in [`concurrent-span-engineering.md`](concurrent-span-engineering.md) §3.

**Go reference:** `golang/go` master `2c3c1caad2` (2026-10-10). Paths are relative to `go/src/`.

**Scope.** Design-level deviations (barrier coverage, stack scanning, allocation stalls, reference processing, and so on) are summarised in [`go-gc-port.md`](go-gc-port.md) §11. This file records the code-level ones: places where a ported function, constant, table or state machine does something different from Go's code. When a code-level deviation changes the design, update §11 as well.

**Not deviations:** using MMTk's integration points (work packets, `Space`, options, side metadata), Rust idioms, and renaming to avoid "Go" in identifiers. Those are expected (engineering §3).

**Effect on the evaluation:** say which metrics could move and in which direction, or "none expected".

| ID | Go code | What Go does | What ConcurrentSpan does | Why | Effect on the evaluation | Our code | Design ref |
|---|---|---|---|---|---|---|---|
| | | | | | | | |

No code-level deviations yet: no Go code has been ported.

## Details

Longer notes for entries that need them. Use the entry's ID as the heading.
