# Glossary

This document explains basic concepts of garbage collection.  MMTk uses those terms as described in
this document.  Different VMs may define some terms differently.  Should there be any confusion,
this document will help disambiguating them.  We use the book [*The Garbage Collection Handbook: The
Art of Automatic Memory Management*][GCHandbook] as the primary reference.

[GCHandbook]: https://gchandbook.org/

## Object graph

Object graph is a graph-theory view of the garbage-collected heap.  An **object graph** is a
directed graph that contains *nodes* and *edges*.  An edge always points to a node.  But unlike
conventional graphs, an edge may originate from either another node or a *root*.

Each *node* represents an object in the heap.

Each *edge* represents an object reference from an object or a root.  A *root* is a reference held
in a slot directly accessible from [mutators], including local variables, global variables,
thread-local variables, and so on.  A object can have many fields, and some fields may hold
references to objects, while others hold non-reference values.

An object is *reachable* if there is a path in the object graph from any root to the node of the
object.  Unreachable objects cannot be accessed by [mutators].  They are considered
garbage, and can be reclaimed by the garbage collector.

## Mutator

[mutator]: #mutator
[mutators]: #mutator

TODO

## Emergency Collection

Also known as: *emergency GC*

In MMTk, an emergency collection happens when a normal collection cannot reclaim enough memory to
satisfy allocation requests.  Plans may do full-heap GC, defragmentation, etc. during emergency
collections in order to free up more memory.

VM bindings can call `MMTK::is_emergency_collection` to query if the current GC is an emergency GC.
During emergency GC, the VM binding is recommended to retain fewer objects than normal GCs, to the
extent allowed by the specification of the VM or the language.  For example, the VM binding may
choose not to retain objects used for caching.  Specifically, for Java virtual machines, that means
not retaining referents of [`SoftReference`][java-soft-ref] which is primarily designed for
implementing memory-sensitive caches.

[java-soft-ref]: https://docs.oracle.com/en/java/javase/21/docs/api/java.base/java/lang/ref/SoftReference.html

## GC-safe Point

[GC-safe point]: #gc-safe-point
[GC-safe points]: #gc-safe-point

Also known as: *GC-point*

A *GC-safe point* is a place in the code executed by mutators where (stop-the-world) garbage
collection is allowed to happen.

It is impractical to allow GC to happen at any program counter (PC) for several reasons.

1.  The GC needs to identify references held in stack slots and machine registers.  Allowing GC to
    happen at any PC will either force the compiler to generate [stack maps] at all PCs, or force
    the VM to use [conservative stack scanning].
2.  Some operations, such as [write barriers] and [address-based hashing], need to be *atomic with
    respect to GC*.  Allowing GC to happen in the middle of such operations will complicate the
    implementation or make it inefficient.

In practice, we only allow GC to happen at certain *GC-safe points* where the compiler generates
[stack maps].

Note that although concurrent garbage collection can run concurrently with mutators, it also needs
to synchronize with each mutator at a GC-safe point.

Examples of GC-safe points (not exhaustive):

-   [yieldpoints]
-   object allocation (may trigger GC)
-   call sites to other functions where GC is allowed to happen inside

Examples of non-GC-safe points (not exhaustive):

-   [write barriers]
-   [address-based hashing]
-   call sites to other functions during which GC must not happen

For programs without GC semantics (e.g. programs written in C, C++, Rust, etc.), their compilers
(GCC, clang, rustc, ...) are agnostic to GC.  It is up to the VM to decide whether GC can happen
when a mutator is executing native functions written in those languages.  For example, if a mutator
is executing long-running native functions (such as blocking system calls) that cannot access the GC
heap, the VM usually allows GC to happen without waiting for this mutator.  But if a mutator is
running a runtime function that cannot be interrupted by GC (such as the write barrier slow path
implemented as a native function), the VM must wait for the mutator to return from such native
functions before letting GC start.

## Stack Map

[stack map]: #stack-map
[stack maps]: #stack-map

A *stack map* is a data structure that identifies stack slots and registers that may contain
references.  Stack maps are essential for supporting [precise stack scanning].

## Yieldpoint

[yieldpoint]: #yieldpoint
[yieldpoints]: #yieldpoint

Also known as: *GC-check point*

A *yieldpoint* is a point in a program where a mutator checks if it should yield from normal
execution in order to handle certain events, such as garbage collection, profiling, biased lock
revocation, etc.

Compilers of programs with GC semantics (e.g. Java source code or byte code) insert yieldpoints in
various places, such as loop back-edges, so that mutators can yield promptly when GC is triggered
asynchronously by other threads.  Compilers also generate [stack maps] at yieldpoints to make them
[GC-safe points].

Read the paper [*Stop and go: Understanding yieldpoint behavior*][LWB+15] by Lin et al. for more
details.

[LWB+15]: https://dl.acm.org/doi/10.1145/2754169.2754187

## Address-based Hashing

[address-based hashing]: #address-based-hashing

*Address-based hashing* is a GC-assisted space-efficient high-performance method for implementing
identity hash code in copying GC.

Read the [Address-based Hashing](portingguide/concerns/address-based-hashing.md) chapter for more
details.

## Precise Stack Scanning

[precise stack scanning]: #precise-stack-scanning

Also known as: *exact stack scanning*

TODO

## Conservative Stack Scanning

[conservative stack scanning]: #conservative-stack-scanning

TODO

## Write Barrier

[write barrier]: #write-barrier
[write barriers]: #write-barrier

TODO

<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
