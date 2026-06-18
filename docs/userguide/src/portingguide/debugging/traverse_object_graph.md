# Traverse Object Graph with a Debugger

Tracing through an object graph with a debugger can be tricky in MMTk.
Garbage collection in MMTk is executed in units called work packets. These include packets that trace slots and packets that scan objects.
When scanning an object, MMTk may not carry context about the slot from which the object reference was loaded.

This section demonstrates how to use [rr](https://rr-project.org/) to traverse the object graph.
Suppose a program segfaults during GC while scanning a corrupted object (e.g. `0x725a62eb60f0`). Our goal is to determine:
* Which slot loaded this object reference.
* Which object that slot belongs to.

Note: Traversing the entire object graph is rarely the best debugging strategy. Graphs can be large, and resolving
object → slot → object → slot → … repeatedly is time-consuming and error-prone. Prefer simpler approaches if possible.
Object-graph reversal should be considered a last resort.

## Step 1: Set Breakpoints at GC Boundaries

First, replay the execution to the point of the segfault.
Then, to ensure you know which GC cycle you’re in, set breakpoints at the start and end of each GC:

```gdb
(rr) b stop_all_mutators
(rr) b resume_mutators
```

These breakpoints help you detect if replay takes you into a different GC than the one that crashed.

## Step 2: Identify the Slot that Loaded the Object

```admonish note
Older versions of MMTk used `ProcessEdgesWork` and `PlanProcessEdges`.
Current MMTk uses `ProcessSlots` to load object references from slots, and `ProcessNodes` to scan objects.
```

Object references are usually loaded from slots in
[`ProcessSlots::process_slots`](https://github.com/mmtk/mmtk-core/blob/master/src/plan/tracing/gc_work/closure.rs).
The current code looks like this:

```rust
fn process_slots(
    &mut self,
    worker: &mut GCWorker<T::VM>,
    trace: T,
) -> VectorQueue<ObjectReference> {
    let mut queue = VectorObjectQueue::new();

    for slot in self.slots.iter() {
        if let Some(object) = slot.load() {
            let new_object = trace.trace_object(worker, object, &mut queue);
            if T::may_move_objects() && new_object != object {
                slot.store(new_object);
            }
        }
    }

    queue
}
```

At the `slot.load()` / `trace.trace_object(...)` point, `object` is the reference of interest.
To catch the corrupted object (`0x725a62eb60f0`), set a conditional breakpoint there.

Conditions can be expressed in different ways, such as `if object.as_raw_address().as_usize() == 0x725a62eb60f0` (Rust),
`if (uintptr_t)object == 0x725a62eb60f0` (C), and `if $rsi == 0x725a62eb60f0`.

Using registers seems to work more reliably when
the execution keeps switching between Rust and C. You can find which register holds the value `0x725a62eb60f0` (using `info registers`)
and use that as the condition. If the value `0x725a62eb60f0` does not appear in any register, you can `step` forward through one or more statements,
until you see the value appears. Set the breakpoint at that line.

### Automating with a Temporary Breakpoint

Since we only need the breakpoint once, a temporary breakpoint is convenient.
We can also set up a GDB command, as we will likely do this step repeatedly to traverse the object.

Define a helper GDB command.
You need to change the example below to match your recorded trace:
1. The file path.
2. The line number inside `ProcessSlots::process_slots`.
3. The register that holds the value `object`.

```gdb
(rr) define find_slot
>tbreak /path/to/mmtk-core/src/plan/tracing/gc_work/closure.rs:41 if $rsi == $arg0
>reverse-cont
>end
```

Usage:
```gdb
(rr) find_slot 0x725a62eb60f0
Temporary breakpoint 1 at 0x...: /path/to/mmtk-core/src/plan/tracing/gc_work/closure.rs:41. (N locations)
```

If the breakpoint hits (it may take a while), you’ll see something like:
```gdb
Thread 2 hit Temporary breakpoint 1, mmtk::plan::tracing::gc_work::closure::ProcessSlots<...>::process_slots (...) at /path/to/mmtk-core/src/plan/tracing/gc_work/closure.rs:41
41                  let new_object = trace.trace_object(worker, object, &mut queue);
```

Then:
```gdb
(rr) p/x slot
$2 = mmtk_julia::slots::JuliaVMSlot::Simple(mmtk::vm::slot::SimpleSlot {slot_addr: 0x725a62eb6168})
```

Here we learn that object `0x725a62eb60f0` was loaded from slot `0x725a62eb6168`.

If instead you hit `stop_all_mutators`, it means the object was not processed through
`ProcessSlots` (our conditional breakpoint) in this GC. Similar techniques apply: set breakpoints
in other relevant paths until you find where the object comes from.

### Step 3: Identify the Object that Owns the Slot

Now that we know slot `0x725a62eb6168` contains the object `0x725a62eb60f0`, we need to determine which object this slot belongs to.
Slots are reported when bindings scan an object via
[`Scanning::scan_object`](https://docs.mmtk.io/api/mmtk/vm/trait.Scanning.html#tymethod.scan_object),
which calls [`SlotVisitor::visit_slot`](https://docs.mmtk.io/api/mmtk/vm/trait.SlotVisitor.html#tymethod.visit_slot)
for each outgoing reference field.
To find the owning object, set a conditional breakpoint in mmtk-core at the `SlotVisitor`
implementation for closures.

Define another helper command:
```gdb
(rr) define find_object
>tbreak /path/to/mmtk-core/src/vm/scanning.rs:16 if $rsi == $arg0
>reverse-cont
>end
```

Usage:
```gdb
(rr) find_object 0x725a62eb6168
Temporary breakpoint 2 at 0x...: /path/to/mmtk-core/src/vm/scanning.rs:16. (N locations)
```

When the breakpoint hits:
```gdb
Thread 2 hit Temporary breakpoint 2, <... as mmtk::vm::scanning::SlotVisitor<...>>::visit_slot (...) at /path/to/mmtk-core/src/vm/scanning.rs:16
16          self(slot)
```

From the stack, you can walk upward to find the object that is being scanned.
```gdb
(rr) up
#1  ... in <closure at ...>
(rr) up
#2  ... in <your binding>::scan_object(...)
(rr) up
#3  ... in mmtk::plan::tracing::gc_work::closure::ProcessNodes<...>::try_enqueue_slots(...)
(rr) p/x obj
$1 = mmtk::util::address::Address (0x725a62eb6130)
```

By repeating Step 2 (finding the slot that loaded an object) and Step 3 (finding the object that owns that slot), you can walk backward through the object graph. Continue this process until you reach a point of interest -- such as the root object, or an object that erroneously enqueues a slot.
