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
collection is allowed to happen.  Concurrent GC can run concurrently with mutators, but still needs
to synchronize with mutators at GC-safe points.  Regardless, the following statements must be true
when a mutator is at a GC-safe point.

-   References held by a mutator can be identified.  That include references in local variables,
    thread-local variables, and so on.  For compiled code, that include those in stack slots and
    machine registers.
-   The mutator cannot be in the middle of operations that must be *atomic with respect to GC*.
    That includes [write barriers], [address-based hashing], etc.

### Code With GC Semantics

Compilers (including ahead-of-time and just-in-time compilers) for programs with garbage collection
semantics (such as Java source code or bytecode) usually understand GC semantics, too, and can
generate [yieldpoints] and [stack maps] to assist GC.

In practice, such compilers only make certain places in a function GC-safe and only generate [stack
maps] at those places, including but not limited to:

-   [yieldpoints]
-   object allocation sites (may trigger GC)
-   call sites to other functions where GC is allowed to happen inside

If we allow GC to happen at arbitrary PC, it will either force the compiler to generate [stack maps]
at all PCs, or force the VM to use [shadow stacks] or [conservative stack scanning], instead.  It
will also break operations that must be *atomic with respect to GC*, such as [write barrier] and
[address-based hashing].

### Code Without GC Semantics

In contrast, for programs without GC semantics (e.g. programs written in C, C++, Rust, etc.), their
compilers (GCC, clang, rustc, ...) are agnostic to GC.  But many VMs (such as OpenJDK, CRuby, Julia,
etc.) are implemented in such languages.  We don't usually use the term "GC-safe point" for
functions written in C, C++, Rust, etc., but each VM has its own rules to determine whether GC can
happen within functions written in those languages.

Interpreters usually maintain local variables in dedicated stacks or frames data structures.
References in such structures are identified by traversing those stacks or frames, and GC is usually
allowed between bytecode instructions.

Some runtime functions implement operations tightly related to GC, and must be *atomic w.r.t. GC*.
For example, if a function initializes the type information in the header of an object, GC cannot
happen in the middle.  Otherwise the GC will read a corrupted header and crash.  Other examples
include functions that implement the write barrier [slow path] and [address-based hashing].  Such
functions cannot allocate objects, and cannot call any function that may trigger GC.

Some functions do not access the GC heap, or only access the heap in controlled ways (e.g. utilizing
[object pinning], or via safe APIs such as [JNI]).  Some of such functions (such as wrappers for
blocking system calls including `read` and `write`) are long-running.  GC is usually safe when some
mutators are executing such functions.  Compilers for languages with GC semantics usually make *call
sites* to such functions [GC-safe points], and generate [stack maps] at those call sites.  The
runtime usually transitions the state of the current mutator thread so that the GC knows it is in
such a function when requesting all mutators to stop at their next GC-safe points.

[JNI]: https://docs.oracle.com/en/java/javase/21/docs/specs/jni/index.html

## Stack Map

[stack map]: #stack-map
[stack maps]: #stack-map

A *stack map* is a data structure that identifies stack slots and registers that may contain
references.  Stack maps are essential for supporting [precise stack scanning].

## Yieldpoint

[yieldpoint]: #yieldpoint
[yieldpoints]: #yieldpoint

Also known as: *GC-check point*

A *yieldpoint* is a point in a program where a mutator thread checks if it should yield from normal
execution in order to handle certain events, such as garbage collection, profiling, biased lock
revocation, etc.

Compilers of programs with GC semantics (e.g. Java source code and byte code) insert yieldpoints in
various places, such as function epilogues and loop back-edges.  In this way, when GC is triggered
asynchronously by other threads, the current mutator can reach the next yieldpoint quickly and yield
for GC promptly.  Compilers also generate [stack maps] at yieldpoints to make them [GC-safe points].

Because some operations (such as [write barrier]) must be *atomic w.r.t. GC*, [yieldpoints] must not
be inserted in the middle of such operations.

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

## Shadow Stack

[shadow stack]: #shadow-stack
[shadow stacks]: #shadow-stack

TODO

## Write Barrier

[write barrier]: #write-barrier
[write barriers]: #write-barrier

TODO

## Fast Path and Slow Path

[fast path]: #fast-path-and-slow-path
[slow path]: #fast-path-and-slow-path

TODO

## Object Pinning

[object pinning]: #object-pinning

TODO

<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
