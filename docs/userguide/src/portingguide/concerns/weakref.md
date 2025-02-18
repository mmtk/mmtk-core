# Finalizers and Weak References

Some VMs support *finalizers*, *weak references*, and other complex data structures that have weak
reference semantics, such as weak tables (hash tables where the key, the value or both can be weak
references), ephemerons, etc.  The concrete semantics of finalizer and weak reference varies from VM
to VM, but MMTk provides a low-level API that allows the VM bindings to implement their flavors of
finalizer and weak references on top of it.

## Definitions

In this chapter, we use the following definitions.  They may be different from the definitions in
concrete VMs.

**Finalizers** are clean-up operations associated with an object, and are executed when the garbage
collector determines the object is no longer reachable.  Depending on the VM, finalizers may have
different properties.

-   Finalizers may be executed immediately during GC, or postponed to mutator time.
-   They may have access to the object body, or executed independently from the object.
-   They may "resurrect" the unreachable object, or guarantee unreachable objects remain unreachable
    after finalization.

**Weak references** are special [object graph] edges distinct from ordinary "strong" references.

-   An object is *strongly reachable* if there is a path from roots to the object that contains only
    strong references.
-   An object is *weakly reachable* if any path from the roots to the object must contain at least
    one weak reference.

The garbage collector may reclaim weakly reachable objects, clear weak references to weakly
reachable objects, and/or performing associated clean-up operations.

[object graph]: ../../glossary.html#object-graph

**A note for Java programmers**: In Java, the term "weak reference" often refers to instances of
`java.lang.ref.Reference` (including the concrete classes `SoftReference`, `WeakReference`,
`PhantomReference` and the hidden `FinalizerReference` class used by some JVM implementations to
implement finalizers).  Instances of `Reference` are proper Java heap objects, but each instance has
a field that contains a pointer to the referent, and the field can be cleared when the referent
dies.  In this article, we use the term "weak reference" to refer to the pointer inside that field.
In other words, a Java `Reference` instance has a field that holds a weak reference to the referent.

## Overview of MMTk's finalizer and weak reference processing API

During each GC, MMTk core starts tracing from roots.  It will follow strong references discovered by
`Scanning::scan_object` and `Scanning::scan_object_and_trace_edges`.  After all strongly reachable
objects have been reached (i.e. the transitive closure including strongly reachable objects is
computed), MMTk will call `Scanning::process_weak_refs` which is implemented by the VM binding.
Inside this function, the VM binding can do several things.

-   **Query reachability**: The VM binding can query whether any given object has been reached.
    +   Do this with `ObjectReference::is_reachable()`.
-   **Query forwarded address**: If an object has already been reached, the VM binding can further
    query the new address of an object.  This is needed to support copying GC.
    +   Do this with `ObjectReference::get_forwarded_object()`.
-   **Retain objects**: If an object has not been reached at this time, the VM binding can
    optionally demand the object to be retained.  That object *and all descendants* will be kept
    alive during this GC.
    +   Do this with the `tracer_context` argument of `process_weak_refs`.
-   **Request another invocation**: The VM binding can request `Scanning::process_weak_refs` to be
    called again after computing the transitive closure that includes *retained objects and their
    descendants*.  This helps handling multiple levels of weak reference strength.
    +   Do this by returning `true` from `process_weak_refs`.

The `Scanning::process_weak_refs` function also gives the VM binding a chance to perform other
operations, including (but not limited to)

-   **Do clean-up operations**: The VM binding can perform clean-up operations, or queue them to be
    executed after GC.
-   **update fields** that contain weak references.
    -   **Forward the field**: It can write the forwarded address of the referent if moved by a
        copying GC.
    -   **Clear the field**: It can clear the field if the referent has not been reached and the
        binding decides it is unreachable.

Using those primitive operations, the VM binding can support different flavors of finalizers and/or
weak references.  We will discuss common use cases in the following sections.

## Supporting finalizers

Different VMs define "finalizer" differently, but they all involve performing operations when an
object is dead.  The general way to handle finalizer is visiting all **finalizable objects** (i.e.
objects that have associated finalization operations), check if they are unreachable and, if
unreachable, do something about them.

### Identifying finalizable objects

Some VMs determine whether an object is finalizable by its type.  In Java, for example, an object is
finalizable if its `finalize()` method is overridden.  The VM binding can maintain a list of
finalizable objects, and register instances of such types into that list when they are constructed.

Some VMs can dynamically attach finalizing operations to individual objects after objects are
created.  The VM binding can maintain a list of objects with attached finalizers, or maintain a
(weak) hash map that maps finalizable objects to its associated finalizers.

### When to run finalizers?

Depending on the finalizer semantics in different VMs, finalizers can be executed during GC or
during mutator time after GC.

The VM binding can run finalizers immediately in `Scanning::process_weak_refs` when finding a
finalizable object unreachable.  Beware that executing finalizers can be time-consuming.  The VM
binding can creating work packets and let each work packet process a part of all finalizable
objects.  In this way, multiple GC workers can process finalizable objects in parallel.  The
`Scanning::process_weak_refs` function is executed in the `VMRefClosure` stage, so the created work
packets shall be added to the same bucket.

If the finalizers should be executed after GC, the VM binding should enqueue such jobs to
VM-specific queues so that they can be picked up by mutator threads after GC.

### Reading the body of dead object

In some VMs, finalizers can read the fields in dead objects.  Such fields usually include
information needed for cleaning up resources held by the object, such as file descriptors and
pointers to memory not managed by GC.

`Scanning::process_weak_refs` is executed in the `VMRefClosure` stage, which happens after computing
transitive closure, but before any object has been released (which happens in the `Release` stage).
This means the body of all objects, live or dead, can still be accessed during this stage.

Therefore, there is no problem reading the object body if the VM binding executes finalizers
immediately in `process_weak_refs`, or in created work packets in the `VMRefClosure` stage.

However, if the VM needs to execute finalizers after GC, it will be a problem because the object
will have been reclaimed, and memory of the object will have been overwritten by other objects.  In
this case, the VM will need to retain the dead object to make it accessible after the current GC.

### Retaining unreachable objects

Some VMs, particularly the Java VM, executes finalizers during mutator time.  Any finalizable
objects unreachable before a GC must be retained so that they can still be accessed by their
finalizers after the GC.

The `Scanning::process_weak_refs` has an parameter `tracer_context: impl ObjectTracerContext<VM>`.
This parameter provides the necessary mechanism to retain objects and make them (and their
descendants) live through the current GC.  The typical use pattern is:

```rust
{{#include ../../../../../src/vm/tests/mock_tests/mock_test_doc_weakref_code_example.rs:process_weak_refs_finalization}}
```

Within the closure `|tracer| { ... }`, the VM binding can call `tracer.trace_object(object)` to
retain `object`.  It returns the new address of `object` because in a copying GC the `trace_object`
function can also move the object.

Under the hood, `tracer_context.with_tracer` creates a queue and calls the closure.  The `tracer`
implements the `ObjectTracer`  trait, and is just an interface that provides the `trace_object`
method.  Objects retained by `tracer.trace_object` will be enqueued.  After the closure returns,
`with_tracer` will split the queue into reasonably-sized work packets and add them to the
`VMRefClosure` work bucket.  Those work packets will trace the retained objects and their
descendants, effectively expanding the transitive closure to include all objects reachable from the
retained objects.  Because of the overhead of creating queues and work packets, the VM binding
should **retain as many objects as needed in one invocation of `with_tracer`, and avoid calling
`with_tracer` again and again for each object**.

**Don't do this**:

```rust
for object in objects {
    tracer_context.with_tracer(worker, |tracer| { // This is expensive! DON'T DO THIS!
        tracer.trace_object(object);
    });
}
```

Keep in mind that **tracer_context implements the `Clone` trait**.  As introduced in the *When to
run finalizers* section, the VM binding can use work packets to parallelize finalizer processing.
If finalizable objects need to be retained, the VM binding can clone the `trace_context` and give
each work packet a clone of `tracer_context`.

### WARNING: object resurrection

If the VM binding retains an unreachable object for finalization, and the finalizer writes a
reference of that object into a place readable by application threads, including global or static
variable, then the previously unreachable object will become reachable by the application again.
This phenomenon is known as **"resurrection"**, and can be surprising to the programmers.

Developers of VM bindings of existing VMs may have no choice but implementing the finalizer
semantics strictly according to the specification of the VM, even if that would result in
"resurrection".  JVM is a well-known example of the "resurrection" behavior, although the
`Object.finalize()` method has been deprecated for removal, in favor for alternative clean-up
mechanisms such as `PhantomReference` and `Cleaner` which never "resurrect" objects.

Designers of new programming languages or VMs should be aware of the "resurrection" problem.  It is
recommended not to let finalizers have access to the object body.  For finalizers that need to
release certain resources (such as files), the VM may store relevant data (such as file descriptors)
in a separate object and use that as the context of the finalizer.

To avoid unintentionally "resurrecting" objects, if the VM binding intends to get the new address of
a moved object, it should use `object.get_forwarded_object()` instead of
`tracer.trace_object(object)`, although the latter also returns the new address if `object` is
already moved.


## Supporting weak references

The general way to handle weak references is, after computing the transitive closure, iterate
through all fields that contain weak references to objects.  For each field,

-   if the referent has already been reached, write the new address of the object to the field (or
    do nothing if the object is not moved);
-   otherwise, clear the field, writing `null`, `nil`, or whatever represents a cleared weak
    reference to the field.

### Identifying weak references

Weak references in fields of *global* (per-VM) data structures are relatively straightforward.  We
just need to enumerate them in `Scanning::process_weak_refs`.

There are also fields in *heap objects* that hold weak references to other heap objects.  There are
two basic ways to identify them.

-   **Register on creation**: We may record objects that contain weak reference fields in a global
    list when such objects are created.  In `Scanning::process_weak_refs`, we just need to iterate
    through this list, process the fields, and remove dead objects from the list.
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
semantics, we may choose to execute the clean-up operations immediately, or enqueue them to be
executed after GC.  We may retain the unreachable referent if we need to.

### Soft references

Java has a special kind of weak reference: `SoftReference`.  The API allows the GC to choose between
(1) retaining softly reachable referents, and (2) clearing references to softly reachable objects.
When using MMTk, there are two ways to implement this semantics.

The easiest way is **treating `SoftReference` as strong references in non-emergency GCs, and
treating them like `WeakReference` in emergency GCs**.

-   During non-emergency GC, we let `Scanning::scan_object` and
    `Scanning::scan_object_and_trace_edges` scan the weak reference field inside a `SoftReference`
    instance as if it were an ordinary strong reference field.  In this way, softly reachable
    objects will be included in the (strong) transitive closure from roots.  By the first time
    `Scanning::process_weak_refs` is called, strongly reachable objects will have already been
    reached (i.e. `object.is_reachable()` will be true).  They will be kept alive just like strongly
    reachable objects.
-   During emergency GC, however, skip this field in `Scanning::scan_object` or
    `Scanning::scan_object_and_trace_edges`, and clear `SoftReference` just like `WeakReference` in
    `Scanning::process_weak_refs`.  In this way, softly reachable objects will become unreachable
    unless they are subject to finalization.

The other way is **retaining referents of `SoftReference` after the strong closure**.  This involves
supporting multiple levels of reference strengths, which will be introduced in the next section.

### Multiple levels of reference strength

Some VMs support multiple levels of weak reference strengths.  Java, for example, has
`SoftReference`, `WeakReference`, `FinalizerReference` (internal) and `PhantomReference`, in the
order of decreasing strength.

This can be supported by running `Scanning::process_weak_refs` multiple times.  If
`process_weak_refs` returns `true`, it will be called again after all pending work packets in the
`VMRefClosure` stage has been executed.  Those pending work packets include all work packets that
compute the transitive closure from objects retained during `process_weak_refs`.  This allows the VM
binding to expand the transitive closure multiple times, each handling weak references at different
levels of strength.

Take Java as an example,  we may run `process_weak_refs` four times.

1.  Visit all `SoftReference`.
    -   If the referent has been reached, then
        -   forward the referent field.
    -   If the referent has not been reached, yet, then
        -   if it is not emergency GC, then
            -   retain the referent and update the referent field.
        -   it it is emergency GC, then
            -   clear the referent field,
            -   remove the `SoftReference` from the list of soft references, and
            -   optionally enqueue it to the associated `ReferenceQueue` if it has one.
    -   (This step may expand the transitive closure in emergency GC if any referents are retained.)
2.  Visit all `WeakReference`.
    -   If the referent has been reached, then
        -   forward the referent field.
    -   If the referent has not been reached, yet, then
        -   clear the referent field,
        -   remove the `WeakReference` from the list of weak references, and
        -   optionally enqueue it to the associated `ReferenceQueue` if it has one.
    -   (This step cannot expand the transitive closure.)
3.  Visit the list of finalizable objects.
    -   If the finalizable object has been reached, then
        -   forward the reference in the list.
    -   If the finalizable object has not been reached, yet, then
        -   retain the finalizable object, and
        -   remove it from the list of finalizable objects, and
        -   enqueue it for finalization.
    -   (This step may expand the transitive closure if any finalizable objects are retained.)
4.  Visit all `PhantomReference`.
    -   If the referent has been reached, then
        -   forward the referent field.
        -   (Note: `PhantomReference#get()` always returns `null`, but the actual referent field
            shall hold a valid reference to the referent before it is cleared.)
    -   If the referent has not been reached, yet, then
        -   clear the referent field,
        -   remove the `PhantomReference` from the list of phantom references, and
        -   optionally enqueue it to the associated `ReferenceQueue` if it has one.
    -   (This step cannot expand the transitive closure.)

As an optimization,

-   Step 1 can be, as we described in the previous section, eliminated by merging it with the strong
    closure in non-emergency GC, or with `WeakReference` processing in emergency GC.
-   Step 2 can be merged with Step 3 since Step 2 never expands the transitive closure.

Therefore, we only need to run `process_weak_refs` twice:

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

An [Ephemeron] has a *key* and a *value*, both of which are object references.  The key is a weak
reference, while the value keeps the referent alive only if both the ephemeron itself and the key
are reachable.

[Ephemeron]: https://dl.acm.org/doi/10.1145/263700.263733

To support ephemerons, the VM binding needs to identify ephemerons.  This includes ephemerons as
individual objects, objects that contain ephemerons, and, equivalently, objects that contain
key/value fields that have semantics similar to ephemerons.

The following is the algorithm for processing ephemerons.  It gradually discovers ephemerons as we
do the tracing.  We maintain a queue of ephemerons which is empty before the `Closure` stage.

1.  In `Scanning::scan_object` and `Scanning::scan_object_and_trace_edges`, we enqueue ephemerons as
    we scan them, but do not trace either the key or the value fields.
2.  In `Scanning::process_weak_refs`, we iterate through all ephemerons in the queue.  If the key of
    an ephemeron has been reached, but its value has not yet been reached, then retain its value,
    and remove the ephemeron from the queue.  Otherwise, keep the object in the queue.
3.  If any value is retained, return `true` from `Scanning::process_weak_refs` so that it will be
    called again after the transitive closure from retained values are computed.  Then go back to
    Step 2.
4.  If no value is retained, the algorithm completes.  The queue contains reachable ephemerons that
    have unreachable keys.

This algorithm can be modified if we have a list of all ephemerons before GC starts.  We no longer
need to maintain the queue.

-   In Step 1, we don't need to enqueue ephemerons.
-   In Step 2, we iterate through all ephemerons.  We retain the value if both the ephemeron itself
    and the key have been reached, and the value has not been reached, yet.  We don't need to remove
    any ephemeron from the list.
-   When the algorithm completes, we can identify both reachable and unreachable ephemerons that
    have unreachable keys.  But we need to remove unreachable (dead) ephemerons from the list
    because they will be recycled in the `Release` stage.

And we can go through ephemerons with unreachable keys and do necessary clean-up operations, either
immediately or postponed to mutator time.


## Optimizations

### Generational GC

MMTk provides generational GC plans.  Currently, there are `GenCopy`, `GenImmix` and `StickyImmix`.
In a minor GC, a generational plan only consider *young objects* (i.e. objects allocated since the
last GC) as candidates of garbage, and will assume all *old objects* (i.e. objects survived the last
GC) are live.

The VM binding can query if the current GC is a nursery GC by calling

```rust
let is_nursery_gc = mmtk.get_plan().generational().is_some_and(|gen|
    gen.is_current_gc_nursery());
```

The VM binding can make use of this information when processing finalizers and weak references.  In
a minor GC,

-   The VM binding only needs to visit **finalizable objects allocated since the last GC**.  Other
    finalizable objects must be old and will not be considered dead.
-   The VM binding only needs to visit **weak reference slots written since the last GC**.  Other
    slots must be pointing to old objects (if not `null`).  For weak hash tables, if existing
    entries are immutable, it is sufficient to only visit newly added entries.

Implementation-wise, the VM binding can split the lists or hash tables into two parts: one for old
entries and another for young entries.

### Copying versus non-copying GC

MMTk provides both copying and non-copying GC plans.  `MarkSweep` never moves any objects.
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
still recommend VMs to implement *address-based hashing* if possible.  In that case, we never need
to rehash any hash tables due to object movement.

```admonish info
When using **address-based hashing**, the hash code of an object depends on whether its hash code
has been observed before, and whether it has been moved after its hash code has been observed.

-   If never observed, the hash code of an object will be its current address.
-   When the object is moved the first time after its hash code is observed, the GC thread copies
    its old address to a field of the new copy.  From then on, its hash code will be read from that
    field.
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
