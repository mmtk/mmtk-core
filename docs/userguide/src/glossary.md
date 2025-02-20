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
in a slot directly accessible from [mutators][mutator], including local variables, global variables,
thread-local variables, and so on.  A object can have many fields, and some fields may hold
references to objects, while others hold non-reference values.

An object is *reachable* if there is a path in the object graph from any root to the node of the
object.  Unreachable objects cannot be accessed by [mutators][mutator].  They are considered
garbage, and can be reclaimed by the garbage collector.

[mutator]: #mutator

## Mutator

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

<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
