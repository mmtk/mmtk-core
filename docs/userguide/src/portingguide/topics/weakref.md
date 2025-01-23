# Finalizers and Weak References

Some VMs support **finalizers**.  In simple terms, finalizers are clean-up operations associated
with an object, and are executed when the object is dead.

Some VMs support **weak references**.  If an object cannot be reached from roots following only
strong references, the object will be considered dead.  Weak references to dead objects will be
cleared, and associated clean-up operations will be executed.  Some VMs also support more complex
weak data structures, such as weak hash tables, where keys, values, or both, can be weak references.

The concrete semantics of finalizer and weak reference varies from VM to VM, but MMTk provides a
low-level API that allows the VM bindings to implement their flavours of finalizer and weak
references on top of it.

**A note for Java programmers**: In Java, the term "weak reference" often refers to instances of
`java.lang.ref.Reference` (including the concrete classes `SoftReference`, `WeakReference`,
`PhantomReference` and the hidden `FinalizerReference` class used by some JVM implementations to
implement finalizers).  Instances of `Reference` are proper Java heap objects, but each instance has
a field that contains a pointer to the referent, and the field can be cleared when the referent
dies.  In this article, we use the term "weak reference" to refer to the pointer inside that field.
In other words, a Java `Reference` instance has a field that holds a weak reference to the referent.

## Overview

During each GC, after the transitive closure is computed, MMTk calls `Scanning::process_weak_refs`
which is implemented by the VM binding.  Inside this function, the VM binding can do several things.

-   **Query reachability**: The VM binding can query whether any given object has been reached in
    the transitive closure.
    -   **Query forwarded address**: If an object is already reached, the VM binding can further
        query the new address of an object.  This is needed to support copying GC.
    -   **Resurrect object**: If an object is not reached, the VM binding can optionally resurrect
        the object.  It will keep that object *and all descendants* alive.
-   **Request another invocation**: The VM binding can request `Scanning::process_weak_refs` to be
    *called again* after computing the transitive closure that includes *resurrected objects and
    their descendants*.  This helps handling multiple levels of weak reference strength.

Concretely,

-   `ObjectReference::is_reachable()` queries reachability,
-   `ObjectReference::get_forwarded_object()` queries forwarded address, and
-   the `tracer_context` argument provided by the `Scanning::process_weak_refs` function can
    resurrect objects.
-   Returning `true` from `Scanning::process_weak_refs` will make it called again.

The `Scanning::process_weak_refs` function also gives the VM binding a chance to perform other
operations, including (but not limited to)

-   **Do clean-up operations**: The VM binding can perform clean-up operations, or queue them to be
    executed after GC.
-   **update fields** that contain weak references.
    -   **Forward the field**: It can write the forwarded address of the referent if moved by a
        copying GC.
    -   **Clear the field**: It can clear the field if the referent is unreachable.

Using those primitive operations, the VM binding can support different flavours of finalizers and/or
weak references.  We will discuss different use cases in the following sections.

## Supporting finalizers

Different VMs define "finalizer" differently, but they all involve performing operations when an
object is dead.  The general way to handle finalizer is visiting all **finalizable objects** (i.e.
objects that have associated finalization operations), check if they are dead and, if dead, do
something about them.

### Identifying finalizable objects

Some VMs determine whether an object is finalizable by its type.  In Java, for example, an object is
finalizable if its `finalize()` method is overridden.  We can register instances of such types when
they are constructed.

Some VMs can attach finalizing operations to an object after it is created.  The VM can maintain a
list of objects with attached finalizers, or maintain a (weak) hash map that maps finalizable
objects to its associated finalizers.

### When to run finalizers?

Depending on the semantics, finalizers can be executed during GC or during mutator time after GC.

The VM binding can run finalizers in `Scanning::process_weak_refs` after finding a finalizable
object dead.  But beware that MMTk is usually run with multiple GC workers.  The VM binding can
parallelise the operations by creating work packets.  The `Scanning::process_weak_refs` function is
executed in the `VMRefClosure` stage, so the created work packets shall be added to the same bucket.

If the finalizers should be executed after GC, the VM binding should enqueue them to VM-specific
queues so that they can be picked up after GC.

### Reading the body of dead object

In some VMs, finalizers can read the fields in dead objects.  Such fields usually include
information needed for cleaning up resources held by the object, such as file descriptors and
pointers to memory or objects not managed by GC.

`Scanning::process_weak_refs` is executed in the `VMRefClosure` stage, which happens after the
strong transitive closure (including all objects reachable from roots following only strong
references) has been computed, but before any object has been released (which happens in the
`Release` stage).  This means the body of all objects, live or dead, can still be accessed during
this stage.

Therefore, if the VM needs to execute finalizers during GC, the VM binding can execute them in
`process_weak_refs`, or create work packets in the `VMRefClosure` stage.

However, if the VM needs to execute finalizers after GC, there will be a problem because the object
will be reclaimed, and memory of the object will be overwritten by other objects.  In this case, the
VM will need to "resurrect" the dead object.

### Resurrecting dead objects

Some VMs, particularly the Java VM, executes finalizers during mutator time.  The dead finalizable
objects must be brought back to life so that they can still be accessed after the GC.

The `Scanning::process_weak_refs` has an parameter `tracer_context: impl ObjectTracerContext<VM>`.
This parameter provides the necessary mechanism to resurrect objects and make them (and their
descendants) live through the current GC.  The typical use pattern is:

```rust
impl<VM: VMBinding> Scanning<VM> for VMScanning {
    fn process_weak_refs(
        worker: &mut GCWorker<VM>,
        tracer_context: impl ObjectTracerContext<VM>,
    ) -> bool {
        let finalizable_objects = ...;
        let mut new_finalizable_objects = vec![];

        tracer_context.with_tracer(worker, |tracer| {
            for object in finalizable_objects {
                if object.is_reachable() {
                    // Object is still alive, and may be moved if it's copying GC.
                    let new_object = object.get_forwarded_object().unwrap_or(object);
                    new_finalizable_objects.push(new_object);
                } else {
                    // Object is dead.  Resurrect it.
                    let new_object = tracer.trace_object(object);
                    enqueue_finalizable_object_to_be_executed_later(new_object);
                }
            }
        });

        // more code ...
    }
}
```

The `tracer` parameter of the closure is an `ObjectTracer`.  It provides the `trace_object` method
which resurrects an object and returns the forwarded address.

`tracer_context.with_tracer` creates a temporary `ObjectTracer` instance which the VM binding can
use within the given closure.  Objects resurrected by `trace_object` in the closure are enqueued.
After the closure returns, `with_tracer` will create reasonably-sized work packets for tracing the
resurrected objects and their descendants.  Therefore, the VM binding is encouraged use one
`with_tracer` invocation to resurrect as many objects as needed.  Do not call `with_tracer` too
often, or it will create too many small work packets, which hurts the performance.

Keep in mind that **`ObjectTracerContext` implements `Clone`**.  If the VM has too many finalizable
objects, it is advisable to split the list of finalizable objects into smaller chunks.  Create one
work packets for each chunk, and give each work packet a clone of `tracer_context` so that multiple
work packets can process finalizable objects in parallel.


## Supporting weak references

The general way to handle weak references is, after computing the transitive closure, iterate
through all fields that contain weak references to objects.  For each field,

-   if the referent is already reached, write the new address of the object to the field (or do
    nothing if the object is not moved);
-   otherwise, clear the field, writing `null`, `nil`, or whatever represents a cleared weak
    reference to the field.

### Identifying weak references

Weak references in global slots, including fields of global data structures as well as keys and/or
values in global weak tables, are relatively straightforward.  We just need to enumerate them in
`Scanning::process_weak_refs`.

There are also fields that in heap objects that hold weak references to other heap objects.  There
are two basic ways to identify them.

-   **Register on creation**: We may record objects that contain such fields in a global list when
    such objects are created.  In `Scanning::process_weak_refs`, we just need to iterate through
    this list, process the fields, and remove dead objects from the list.
-   **Discover objects during tracing**: While computing the transitive closure, we scan objects and
    discover objects that contain weak reference fields.  We enqueue such objects into a list, and
    iterate through the list in `Scanning::process_weak_refs` after transitive closure.  The list
    needs to be reconstructed in each GC.

Both methods work, but each has its advantages and disadvantages.  Registering on creation does not
need to reconstruct the list in every GC, while discovering during tracing can avoid visiting dead
objects.  Depending on the nature of your VM, one method may be easier to implement than the other,
especially if your VM's existing GC has already implemented weak reference processing in some way.

### Associated clean-up operations

Some languages and VMs allow certain clean-up operations to be associated with weak references, and
will be executed after the weak reference is cleared.

Such clean-up operations can be supported similar to finalizers.  While we enumerate weak references
in `Scanning::process_weak_refs`, we clear weak references to unreachable objects.  Depending on the
semantics, such as whether the clean-up operation can access the body of unreachable referent, we
may choose to execute the clean-up operation immediately, or enqueue them to be executed after GC,
and may even resurrect the unreachable referent if we need to.

### Soft references

Java has a special kind of weak reference: `SoftReference`.  The API allows the GC to choose whether
to resurrect or clear references to softly reachable objects.  When using MMTk, there are two ways
to implement it.

The easiest way is **treating `SoftReference` as strong references in non-emergency GCs, and
treating them as weak references in emergency GCs**.  During non-emergency GC, we let
`Scanning::scan_objects` scan the weak reference field inside a `SoftReference` instance as if it
were an ordinary strong reference field.  In this way, the (strong) transitive closure after the
`Closure` stage will also include softly reachable objects, and they will be resurrected.  During
emergency GC, however, skip this field in `Scanning::scan_objects`, and clear `SoftReference` just
like `WeakReference` in `Scanning::process_weak_refs`.  In this way, softly reachable objects will
be dead if not subject to finalization.

The other way is **resurrecting referents of `SoftReference` after the strong closure**.  This
involves supporting multiple levels of reference strengths, which will be introduced in the next
section.

### Multiple levels of reference strength

Some VMs support multiple levels of weak reference strengths.  Java, for example, has
`SoftReference`, `WeakReference`, `FinalizerReference` (internal) and `PhantomReference`, in the
order of decreasing strength.  

This can be supported by running `Scanning::process_weak_refs` multiple times.  If
`process_weak_refs` returns `true`, it will be called again after all pending work packets in the
`VMRefClosure` stage has been executed.  That include all work packets that compute the transitive
closure from objects resurrected during `process_weak_refs`.  This allows the VM binding to expand
the transitive closure multiple times, each handling weak references at different levels of
reachability.

Take Java as an example,  we may run `process_weak_refs` four times.

1.  Visit all `SoftReference`.
    -   If the referent is reachable, then
        -   forward the referent field.
    -   If the referent is unreachable, choose between one of the following:
        -   Resurrect the referent and update the referent field.
        -   Clear the referent field, remove the `SoftReference` from the list of soft references,
            and optionally enqueue it to the associated `ReferenceQueue` if it has one.
    -   (This step may expand the transitive closure if any referents are resurrected.)
2.  Visit all `WeakReference`.
    -   If the referent is reachable, then
        -   forward the referent field.
    -   If the referent is unreachable, then
        -   clear the referent field, remove the `WeakReference` from the list of weak references,
            and optionally enqueue it to the associated `ReferenceQueue` if it has one.
    -   (This step cannot expand the transitive closure.)
3.  Visit the list of finalizable objects (may be implemented as `FinalizerReference` by some JVMs).
    -   If the finalizable object is reachable, then
        -   forward the reference to it since it may have been moved.
    -   If the finalizable object is unreachable, then
        -   remove it from the list of finalizable objects, and enqueue it for finalization.
    -   (This step may expand the transitive closure if any finalizable objects are resurrected.)
4.  Visit all `PhantomReference`.
    -   If the referent is reachable, then
        -   forward the referent field.  (Note: `PhantomReference#get()` always returns `null`, but
            the actual referent field shall hold a valid reference to the referent.)
    -   If the referent is unreachable, then
        -   clear the referent field, remove the `PhantomReference` from the list of phantom
            references, and optionally enqueue it to the associated `ReferenceQueue` if it has one.
    -   (This step cannot expand the transitive closure.)

As an optimization, Step 1 can be eliminated by merging it with the strong closure in non-emergency
GC, or with `WeakReference` processing in emergency GC, as we described in the previous section.
Step 2 can be merged with Step 3 since Step 2 never expands the transitive closure.  Therefore, we
only need to run `process_weak_refs` twice:

1.  Handle `WeakReference` (and also `SoftReference` in emergency GC), and then handle finalizable
    objects.
2.  Handle `PhandomReference`.

To implement this, the VM binding may need to implement some kind of *state machine* so that the
`Scanning::process_weak_refs` function behaves differently each time it is called.  For example,

```rust
fn process_weak_ref(...) -> bool {
    let mut state = /* Get VM-specific states here. */;

    match *state {
        State::ProcessSoftReference => {
            process_soft_references(...);
            *state = State::ProcessWeakReference;
            return true; // Run this function again.
        }
        State::ProcessWeakReference => {
            process_weak_references(...);
            *state = State::ProcessFinalizableObjects;
            return true; // Run this function again.
        }
        State::ProcessFinalizableObjects => {
            process_finalizable_objects(...);
            *state = State::ProcessPhantomReferences;
            return true; // Run this function again.
        }
        State::ProcessPhantomReferences => {
            process_phantom_references(...);
            *state = State::ProcessSoftReference
            return false; // Proceed to the Release stage.
        }
    }

}
```

### Ephemerons

TODO


## Optimizations

### Generational GC

MMTk provides generational GC plans.  Currently, there are `GenCopy`, `GenImmix` and `StickyImmix`.
In a minor GC, a generational plan only consider *young objects* (i.e. objects allocated since the
last GC) as candidates of garbage, and will assume all *old objects* (i.e. objects survived the last
GC) are live.

The VM binding can query if the current GC is a nursery GC by calling

```rust
let is_nursery_gc = mmtk.get_plan().is_some_and(|gen| gen.is_current_gc_nursery());
```

The VM binding can make use of this information when processing finalizers and weak references.  In
a minor GC,

-   The VM binding only needs to visit **finalizable objects allocated since the last GC**.  Other
    finalizable objects must be old and will not be considered dead.
-   The VM binding only needs to visit **weak reference slots written since the last GC**.  Other
    slots must be pointing to old objects (if not `null`).  For weak hash tables, if existing
    entries are immutable, it is sufficient to visit newly added entires.

Implementation-wise, the VM binding can split the list or hash tables into two parts: one for old
entries and another for young entries.

### Copying versus non-copying GC

During non-copying GC, objects will not be moved.  In MMTk, `MarkSweep` never moves any objects.
`MarkCompact`, `SemiSpace` always moves all objects.  Immix-based plans sometimes do non-copying GC,
and sometimes do copying GC.  Regardless of the plan, the VM binding can query if the current GC is
a copying GC by calling

```rust
let may_move_object = mmtk.get_plan().current_gc_may_move_object();
```

If it returns `false`, the current GC will not move any object.

The VM binding can make use of this information.  For example, if a weak hash table uses object
addresses as keys, and the hash code is computed directly from the address, then the VM will need to
rehash the table during copying GC because changing the address may move the entry to a different
hash bin.  But if the current GC is non-moving, the VM binding will not need to rehash the table,
but only needs to remove entries for dead objects.  Despite of this optimization opportunity, we
still recommend VMs to implement *address-based hashing* if possible.

```admonish info
When using **address-based hashing**, the hash code of an object depends on whether its hash code
has been observed before, and whether it has been moved after its hash code has been observed.

-   If never observed, the hash code of an object will be its current address.
-   When the object is moved the first time after its hash code is observed, the GC thread copy its
    old address to a field of the new copy.  Its hash code will be read from that field.
-   When such an object is copied again, its hash code will be copied to the new copy of the object.
    The hash code of the object remains unchanged.

The VM binding needs to implement this in `ObjectModel::copy`.
```

## Deprecated reference and finalizable processors

When porting MMTk from JikesRVM to a dedicated Rust library, we also ported the `ReferenceProcessor`
and the `FinalizableProcessor` from JikesRVM.  They are implemented in mmtk-core, and provide the
mechanisms for handling Java-style soft/weak/phantom references and finalizable objects.  The VM
binding can use those utilities by implementing the `mmtk::vm::ReferenceGlue` and the
`mmtk::vm::Finalizable` traits, and calling the
`mmtk::memory_manager::add_{soft,weak,phantom}_candidate` and the
`mmtk::memory_manager::add_finalizer` functions.

However, those mechanisms are too specific to Java, and are not applicable to most other VMs.  **New
VM bindings should use the `Scanning::process_weak_refs` API**, and we are porting existing VM
bindings away from the built-in reference/finalizable processors.

<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
