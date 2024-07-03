# Extending the timeline tool

This document is mainly for VM binding developers who want to extend this timeline tool to trace and
visualize VM-specific events on the timeline.

## Custom work packets in VM bindings

mmtk-core contains trace points that captures the beginning and end of *all* work packets.  If a VM
bindings defines its own work packets and execute them, they will automatically appear on the
timeline, without needing to modify or extend any scripts.

But if you wish to add additional attributes to work packets or events and browse them in Perfetto
UI, please read on.

## The output format of `capture.bt`

The capturing script `capture.bt` prints events in a text format.  Each line contains
comma-separated values:

```
name,ph,tid,timestamp,arg0,arg1,arg2,...
```

The `visualize.py` script will transform those lines into the [Trace Event Format], a JSON-based
format suitable for Perfetto UI, like this: `{"name": name, "ph": ph, "tid": tid, "ts": ts}`.
Possible values of the event type (or "phase", "ph") are defined by the [Trace Event Format].  For
example, "B" and "E" represent the beginning and the end of a duration event, while "i" represents
an instant event.  Additional arguments (arg0, arg1, ...) are processed by `visualize.py` in ways
specific to each event.  Some arguments are added to the resulting JSON object, for example
`{"name": name, ..., "args": {"is_roots": 1, "num_slots": 2}}`  The data in "args" are
human-readable, and can be displayed on Perfetto UI.

[Trace Event Format]: https://docs.google.com/document/d/1CvAClvFfyA5R-PhYUmn5OOQtYMH4h6I0nSsKchNAySU

## Extending the capturing script

VM binding developers may insert custom USDT trace points into the VM binding.  They need to be
captured by bpftrace.

The `capture.py` can use the `-x` command line option to append a script after `capture.bt` which is
the base bpftrace script used by `capture.py`.

For example, you can hack the [mmtk-openjdk](https://github.com/mmtk/mmtk-openjdk) binding and add a
dependency to the `probe` crate in `Cargo.toml`.

```toml
probe = "0.5"
```

Then add the following `probe!` macro in `scan_vm_specific_roots` in `scanning.rs`:

```rust
    probe!(mmtk_openjdk, hello);
```

and create a bpftrace script `capture_openjdk.bt`:

```c
usdt:$MMTK:mmtk_openjdk:hello {
    if (@enable_print) {
        printf("hello,i,%d,%lu\n", tid, nsecs);
    }
}
```

and use the `-x` command line option while invoking `capture.py`:

```shell
./capture.py -x capture_openjdk.bt -m /path/to/libmmtk_openjdk.so ... > output.log
```

and run a benchmark with OpenJDK (such as `lusearch`).  Use the unmodified `visualize.py` to process
the log, and you will see an arrow representing the "hello" event somewhere on the timeline.  It
should be quite obvious because it will be a lone instant event right below the
`ScanVMSpecificRoots` work packet.

## Extending the visualization script

The `visualize.py` script also allows extension using the `-x` command line option.  The `-x` option
points to a Python script that implements two functions: `enrich_event_extra` and
`enrich_meta_extra` (you may omit either one if you don't need).  `enrich_event_extra` is used to
process events that the `visualize.py` script doesn't recognize.  We'll cover `enrich_meta_extra`
later.

For example, modify the `probe!` macro and add an argument:

```rust
    probe!(mmtk_openjdk, hello, 42);
```

and modify `capture_openjdk.bt` to print `arg0` in the CSV:

```c
        printf("hello,i,%d,%lu,%lu\n", tid, nsecs, arg0);
```

and create a Python script `visualize_openjdk.py`:

```python
def enrich_event_extra(log_processor, name, ph, tid, ts, result, rest):
    match name:
        case "hello":
            result["args"] |= {
                "the_number": int(rest[0]),
            }
```

Process the log with `visualize.py`, adding a `-x` option:

```shell
./visualize.py -x visualize_openjdk.py output.log
```

Load the output into Perfetto UI and select the hello event, and you shall see the "the_number"
argument in the "Arguments" block on the right side of the "Current Selection" panel.

## Meta events

The `capture.bt` script and its extensions can print events with type "meta" instead of the usual
"B", "E", "i", etc.  "meta" is not a valid event type defined by the [Trace Event Format].  While
going through the log, the `visualize.py` script remembers the current work packet each thread is
executing.  When `visualize.py` sees a "meta" event, it will find the "B" event for the current work
packet of the current thread, and modifies it, usually by adding more arguments to the event using
information (arguments) provided by the "meta" event.  For example, the `process_slots` "meta" event
will "patch" the work packet event with additional arguments to display the number of slots
processed.

Users can extend `visualize.py` and define the `enrich_meta_extra` function to handle "meta" events
the `visualize.py` script doesn't recognize.

For example, hack the mmtk-openjdk binding again, and add the following `probe!` macro into
`scan_roots_in_mutator_thread` in `scanning.rs`:

```rust
        probe!(mmtk_openjdk, hello2, 43);
```

Capture the event in `capture_openjdk.bt`:

```c
usdt:$MMTK:mmtk_openjdk:hello2 {
    if (@enable_print) {
        printf("hello2,meta,%d,%lu,%lu\n", tid, nsecs, arg0);
    }
}
```

Process the meta event in `visualize_openjdk.py`:

```python
def enrich_meta_extra(log_processor, name, tid, ts, current, rest):
    match name:
        case "hello2":
            current["args"] |= {
                "the_number": int(rest[0]),
            }
```

Run a benchmark, capture a log (with `-x capture_openjdk.bt`) and visualize it (with `-x
visualize_openjdk.py`).  Open Perfetto UI and click on a `ScanMutatorRoots` work packet, and you
will see the `the_number` argument.

## Notes

bpftrace may drop events, so it may fail to record the beginning of some work packets.  This affects
work packets defined in both mmtk-core and the VM binding.  If this happens, `visualize.py` may see
some "meta" events on threads which are apparently not executing any work packets.  Such "meta"
events are silently ignored.

<!--
vim: ts=4 sw=4 sts=4 et tw=100
-->
