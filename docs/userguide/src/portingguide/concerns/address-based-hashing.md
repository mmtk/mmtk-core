# Address-based Hashing

Address-based hashing is a GC-assisted method for implementing identity hash code in copying GC.  It
has the advantage of high performance and not requiring a dedicated hash field for most objects.

This chapter is especially useful for VMs that previously used non-moving GC and naively used the
object address as identity hash code.

## Concepts

An **identity hash code** of an object is a hash code that never changes during the lifetime of the
object, i.e. since it is allocated, until it is reclaimed by the GC.

-   It is unrelated to the *value* the object represents.  This means modifying the fields of an
    object does not modify its identity hash code.
-   Copying GC does *not* change object identities.  If an object is moved by a copying GC, its
    identity hash code remains the same.
-   It is not required to be *unique*.  Two different objects are allowed to have the same hash
    code.

For non-copying GC algorithms, the *address* of an object never changes, and is therefore an ideal
identity hash code.

For copying GC algorithms, however, we cannot simply use the address of an object because it will be
changed when the GC moves the object.  A naive solution is adding an extra field to every object to
hold its hash code, and the field is copied when the GC moves the object.  Although this approach
works (and it is used by real-world VMs such as OpenJDK), it has obvious drawbacks.  It
unconditionally adds a field to every object.  However,

-   **Objects are rarely moved** in advanced GC algorithms such as Immix.
-   **Few objects ever have identity hash code _observed_** (e.g.  by calling
    `System.identityHashCode(object)` in Java) in real-world applications.  According to [the
    Lilliput project of the OpenJDK][openjdk-lilliput], with most workloads, only relatively few
    (<1%) Java objects are ever assigned an identity hash.

[openjdk-lilliput]: https://wiki.openjdk.org/display/lilliput

**Address-based hashing** solves the problem by not adding the extra hash field until both of the
following conditions are true:

1.  The identity hash code of the object has been observed, and
2.  the object is moved by the GC *after* its identity hash code has been observed.

Under the *weak generational hypothesis*, i.e. most objects die young, most objects won't live long
enough until the extra hash field becomes necessary.

The address-based hashing algorithm is implemented [in JikesRVM][jikesrvm-hash], and is planned to
be implemented in OpenJDK ([the Lilliput project][lilliput-ihash]).

[jikesrvm-hash]: https://www.jikesrvm.org/JavaDoc/org/jikesrvm/objectmodel/JavaHeader.html
[lilliput-ihash]: https://wiki.openjdk.org/display/lilliput/Compact+Identity+Hashcode

## The Address-based Hashing Algorithm

Each object is in one of the three states:

-   `Unhashed`
-   `Hashed`
-   `HashedAndMoved`

The state-transition graph is shown below:

```
   move           hash          move or hash  
   ┌──┐           ┌──┐              ┌──┐      
┌──▼──┴──┐ hash ┌─▼──┴─┐ move ┌─────▼──┴─────┐
│Unhashed├─────►│Hashed├─────►│HashedAndMoved│
└────────┘      └──────┘      └──────────────┘
```

States are transitioned upon events labelled on the edges:

-   `hash`: The mutator observes the identity hash code of an object.
-   `move`: The GC moves the object.

An object starts in the `Unhashed` state when allocated.  The GC is free to move it any times, and
its state remains `Unhashed`, as long as its identity hash code is not observed.

When the identity hash code is observed for the first time, its state is changed to `Hashed`.  **In
the `Hashed` state, the identity hash code of an object is its address.**  The object will continue
to use its address as its identity hash code until the object is moved.

When a `Hashed` object is moved, the GC adds a hash field (distinct from high-level language fields
defined by the application) to the new copy of the object, and writes its old address into that
field.  The state of the object is transitioned to `HashedAndMoved`.  **In the `HashedAndMoved`
state, the identity hash code of an object is the value in its added hash field**, and it will keep
using the value in the field as the identity hash code from then on.

When a `HashedAndMoved` object is moved again, the GC copies the hash field to the newer copy, but
no state transition happens.

## Implementing Address-based Hashing with MMTk

Since there are three states, each object needs two bits of metadata to represent that state.  The
two bits are usually held in the object header, but it is also possible to use side metadata albeit
slightly higher memory overhead.

When getting (i.e. observing) the identity hash code of an object, it n


<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
