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
    -   **Retain object**: If an object is not reached, the VM binding can optionally request to
        retain (i.e.  "resurrect") the object.  It will keep that object *and all descendants*
        alive.
-   **Request another invocation**: The VM binding can request `Scanning::process_weak_refs` to be
    *called again* after computing the transitive closure that includes *retained objects and their
    descendants*.  This helps handling multiple levels of weak reference strength.

Concretely,

-   `ObjectReference::is_reachable()` queries reachability,
-   `ObjectReference::get_forwarded_object()` queries forwarded address, and
-   the `tracer_context` argument provided by the `Scanning::process_weak_refs` function can retain
    objects.
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

## Support finalizers

Different VMs define "finalizer" differently, but they all involve performing operations when an
object is dead.  The general way to handle finalizer is visiting all **finalizable objects** (i.e.
objects that have associated finalization operations), check if they are dead and, if dead, do
something about them.

### Identify finalizable objects

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
This parameter provides the necessary mechanism to retain (i.e. "resurrect") objects and make them
(and their descendants) live through the current GC.  The typical use pattern is:

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
                    // Object is dead.  Retain it.
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
which retains an object and returns the forwarded address.

`tracer_context.with_tracer` creates a temporary `ObjectTracer` instance which the VM binding can
use within the given closure.  Objects retained by `trace_object` in the closure are enqueued.
After the closure returns, `with_tracer` will create reasonably-sized work packets for tracing the
retained objects and their descendants.  Therefore, the VM binding is encouraged use one
`with_tracer` invocation to retain as many objects as needed.  Do not call `with_tracer` too often,
or it will create too many small work packets, which hurts the performance.

Keep in mind that **`ObjectTracerContext` implements `Clone`**.  If the VM has too many finalizable
objects, it is advisable to split the list of finalizable objects into smaller chunks.  Create one
work packets for each chunk, and give each work packet a clone of `tracer_context` so that multiple
work packets can process finalizable objects in parallel.


## Support weak references

The general way to handle weak references is, after computing the transitive closure, iterate
through all fields that contain weak references to objects.  For each field,

-   if the referent is already reached, write the new address of the object to the field (or do
    nothing if the object is not moved);
-   otherwise, clear the field, writing `null`, `nil`, or whatever represents a cleared weak
    reference to the field.

### Identify weak references

Weak references in the fields in global data structures, including keys and/or values in global weak
tables, are relatively straightforward.  We just need to enumerate them in
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

### Multiple levels of strength

Some VMs, such as the Java VM support multiple levels of 





<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
