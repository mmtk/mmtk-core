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

Object references are usually loaded from slots in [`ProcessEdgesWork::process_slot`](https://docs.mmtk.io/api/mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html#method.process_slot).
Most MMTk plans use [`PlanProcessEdges` which implements this method like this](https://docs.mmtk.io/api/mmtk/scheduler/gc_work/trait.ProcessEdgesWork.html#method.process_slot):

```rust
fn process_slot(&mut self, slot: SlotOf<Self>) {
    let Some(object) = slot.load() else {
        return;
    };
    let new_object = self.trace_object(object); // Assume this is line 978 in your version
    if P::may_move_objects::<KIND>() && new_object != object {
        slot.store(new_object);
    }
}
```

At line 978, the variable object is the reference of interest.
To catch the corrupted object (`0x725a62eb60f0`), set a conditional breakpoint here.

Conditions can be expressed in different ways, such as `if object.as_raw_address().as_usize() == 0x725a62eb60f0` (Rust),
`if (uintptr_t)object == 0x725a62eb60f0` (C), and `if $rsi == 0x725a62eb60f0`.

Using registers seem to work more reliably when
the execution keeps switching between Rust and C. You can find which register holds the value `0x725a62eb60f0` (using `info registers`)
and use that as the condition. If the value `0x725a62eb60f0` does not appear in any register, you can `step` forward through one or more statements,
until you see the value appears. Set the breakpoint at that line.

### Automating with a Temporary Breakpoint

Since we only need the breakpoint once, a temporary breakpoint is convenient.
We can also set up a GDB command, as we will likely do this step repeatedly to traverse the object.

Define a helper GDB command.
You need to change the example below to match your recorded trace:
1. The file path.
2. The line number of the breakpoint.
3. The register that holds the value `object`.

```gdb
(rr) define find_slot
>tbreak /home/yilin/.cargo/git/checkouts/mmtk-core-3306bdeb8eb4322b/ceea8cf/src/scheduler/gc_work.rs:978 if $rsi == $arg0
>reverse-cont
>end
```

Usage:
```gdb
(rr) find_slot 0x725a62eb60f0
Temporary breakpoint 1 at 0x725a7d4dc2de: /home/yilin/.cargo/git/checkouts/mmtk-core-3306bdeb8eb4322b/ceea8cf/src/scheduler/gc_work.rs:978. (14 locations)
```

If the breakpoint hits (it may take a while), you’ll see something like:
```gdb
Thread 2 hit Temporary breakpoint 1, mmtk::scheduler::gc_work::{impl#39}::process_slot<mmtk_julia::JuliaVM, mmtk::plan::immix::global::Immix<mmtk_julia::JuliaVM>, 1> (self=0x725a64001850, slot=...) at /home/yilin/.cargo/git/checkouts/mmtk-core-3306bdeb8eb4322b/ceea8cf/src/scheduler/gc_work.rs:978
978             let new_object = self.trace_object(object);
```

Then:
```gdb
(rr) p/x slot
$2 = mmtk_julia::slots::JuliaVMSlot::Simple(mmtk::vm::slot::SimpleSlot {slot_addr: 0x725a62eb6168})
```

Here we learn that object `0x725a62eb60f0` was loaded from slot `0x725a62eb6168`.

If instead you hit `stop_all_mutators`, it means the object wasn’t processed through `PlanProcessEdges` (our conditional breakpoint) in this GC. It could have been enqueued by another `ProcessEdgesWork`, by node enqueueing, or it may be a root. Similar techniques apply: set breakpoints in other relevant paths until you find where the object comes from.

### Step 3: Identify the Object that Owns the Slot

Now that we know slot `0x725a62eb6168` contains the object `0x725a62eb60f0`, we need to determine which object this slot belongs to.
Slots are enqueued when bindings scan an object, via [`SlotVisitor::visit_slot`](https://docs.mmtk.io/api/mmtk/vm/trait.SlotVisitor.html#tymethod.visit_slot).
We can set a conditional breakpoint at the implementation of `visit_slot` to capture where the slot is enqueue'd to MMTk.

Define another helper command:
```gdb
(rr) define find_object
>tbreak /home/yilin/.cargo/git/checkouts/mmtk-core-3306bdeb8eb4322b/ceea8cf/src/plan/tracing.rs:61 if $rcx == $arg0
>reverse-cont
>end
```

Usage:
```gdb
(rr) find_object 0x725a62eb6168
Temporary breakpoint 2 at 0x725a7d57a363: /home/yilin/.cargo/git/checkouts/mmtk-core-3306bdeb8eb4322b/ceea8cf/src/plan/tracing.rs:61. (3 locations)
```

When the breakpoint hits:
```gdb
Thread 2 hit Temporary breakpoint 2, mmtk::plan::tracing::VectorQueue<mmtk_julia::slots::JuliaVMSlot>::push<mmtk_julia::slots::JuliaVMSlot> (self=0x725a695fafe0, v=...) at /home/yilin/.cargo/git/checkouts/mmtk-core-3306bdeb8eb4322b/ceea8cf/src/plan/tracing.rs:61
61              if self.buffer.is_empty() {
```

From the stack, you can walk upward to find the object that is being scanned.
```gdb
(rr) up
#1  0x0000725a7d578050 in mmtk::plan::tracing::{impl#4}::visit_slot<mmtk::scheduler::gc_work::PlanProcessEdges<mmtk_julia::JuliaVM, mmtk::plan::immix::global::Immix<mmtk_julia::JuliaVM>, 1>> (self=0x725a695fafe0, slot=...)
    at /home/yilin/.cargo/git/checkouts/mmtk-core-3306bdeb8eb4322b/ceea8cf/src/plan/tracing.rs:125
125             self.buffer.push(slot);
(rr) up
#2  0x0000725a7d536dbb in mmtk_julia::julia_scanning::process_slot<mmtk::plan::tracing::ObjectsClosure<mmtk::scheduler::gc_work::PlanProcessEdges<mmtk_julia::JuliaVM, mmtk::plan::immix::global::Immix<mmtk_julia::JuliaVM>, 1>>> (closure=0x725a695fafe0, slot=...)
    at src/julia_scanning.rs:720
720         closure.visit_slot(JuliaVMSlot::Simple(simple_slot));
(rr) up
#3  mmtk_julia::julia_scanning::scan_julia_obj_n<u8, mmtk::plan::tracing::ObjectsClosure<mmtk::scheduler::gc_work::PlanProcessEdges<mmtk_julia::JuliaVM, mmtk::plan::immix::global::Immix<mmtk_julia::JuliaVM>, 1>>> (obj=..., begin=..., end=..., closure=0x725a695fafe0)
    at src/julia_scanning.rs:111
111             process_slot(closure, slot);
(rr) p/x obj
$1 = mmtk::util::address::Address (0x725a62eb6130)
```

By repeating Step 2 (finding the slot that loaded an object) and Step 3 (finding the object that owns that slot), you can walk backward through the object graph. Continue this process until you reach a point of interest -- such as the root object, or an object that errornously enqueues a slot.
