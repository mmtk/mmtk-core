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


<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
