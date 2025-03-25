# Address-based Hashing

Address-based hashing is a GC-assisted method for implementing identity hash code in copying GC.  It
has the advantage of high performance and not requiring a dedicated hash field for most objects.

This chapter is especially useful for VMs that previously used non-moving GC and naively used the
object address as identity hash code.

## Concepts

An **identity hash code** of an object is a hash code that *never changes* during the lifetime of
the object, i.e. since it is allocated, until it is reclaimed by the GC.

-   It is not required to be *unique*.
    -   Two different objects are allowed to have the same identity hash code.  Like any hashing
        algorithm, collision is allowed.  But a good hash code should be *relatively unique* in
        order to reduce collision.
    -   On the contrary, some programming languages (such as [Python][python-id] and
        [Ruby][ruby-id]) have the notion of *object ID* which is required to be unique.
-   It is unrelated to the *value* the object represents.
    -   For example, modifying a mutable string object does not change its identity hash code.
    -   On the contrary, two string objects that are equal character-wise may (but does not have to)
        have different identity hash code.
-   Copying GC does *not* change object identities.
    -   If an object is moved by a copying GC, its identity hash code remains the same.
    -   On the contrary, when moved by a copying GC, the *address* of the object is changed.

[python-id]: https://docs.python.org/3/library/functions.html#id
[ruby-id]: https://docs.ruby-lang.org/en/master/Object.html#method-i-object_id

For non-copying GC algorithms, the *address* of an object never changes, and all objects have
distinct addresses.  Therefore it is an ideal identity hash code.

For copying GC algorithms, however, we cannot simply use the address of an object because it will be
changed when the GC moves the object.  A naive solution is adding an extra field to every object to
hold its hash code, and the field is copied when the GC moves the object.  Although this approach
works (and it is used by real-world VMs such as OpenJDK), it unconditionally adds a field to every
object.  However,

-   **Objects are rarely moved** in advanced GC algorithms such as Immix.
-   **Few objects ever have identity hash code _observed_** (e.g.  by calling
    `System.identityHashCode(object)` in Java) in real-world applications.  According to [the
    Lilliput project][openjdk-lilliput] of OpenJDK, with most workloads, only relatively few (<1%)
    Java objects are ever assigned an identity hash.

[openjdk-lilliput]: https://wiki.openjdk.org/display/lilliput

**Address-based hashing** solves these problems by not adding the extra hash field until both of the
following conditions are true:

1.  The identity hash code of the object has been observed, and
2.  the object is moved by the GC *after* its identity hash code has been observed.

Under the *weak generational hypothesis* (i.e. most objects die young), most objects will die before
the two conditions become true, and will never need the extra hash field during its lifetime.

The address-based hashing algorithm is implemented [in JikesRVM][jikesrvm-hash], and is planned to
be implemented in OpenJDK ([the Lilliput project][lilliput-ihash]).

[jikesrvm-hash]: https://www.jikesrvm.org/JavaDoc/org/jikesrvm/objectmodel/JavaHeader.html
[lilliput-ihash]: https://wiki.openjdk.org/display/lilliput/Compact+Identity+Hashcode

## The Address-based Hashing Algorithm

Each object is in one of the three **hash code states**:

-   `Unhashed`
-   `Hashed`
-   `HashedAndMoved`

The state-transition graph is shown below:

```
       move                        hash                    move or hash   
     ┌──────┐                    ┌──────┐                    ┌──────┐     
     │      │                    │      │                    │      │     
┌────▼──────┴────┐   hash   ┌────▼──────┴────┐   move   ┌────▼──────┴────┐
│    Unhashed    ├─────────►│     Hashed     ├─────────►│ HashedAndMoved │
└────────────────┘          └────────────────┘          └────────────────┘
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

Here we use the implementation strategy from JikesRVM as an example of how to implement
address-based hashing when using MMTk.  For the best performance, we recommend holding the hash code
state in the object header, and putting the added hash field in the beginning or the end of the
object.  We also introduce alternative strategies in [the next
section](#alternative-implementation-strategies).

### Object Layout

Since there are three states, each object needs two bits of metadata to represent that state.  The
two bits are usually held in the object header.  For example, [in JikesRVM][jikesrvm-hash], the two
hash code state bits are placed after the thin lock bits, as shown below.

```
      TTTT TTTT TTTT TTTT TTTT TTHH AAAA AAAA
 T = thin lock bits
 H = hash code state bits
 A = available for use by GCHeader and/or MiscHeader.
```

The VM also needs to decide the location of the extra hash field.  It is usually placed at the
beginning or at the end of an object, as shown in the diagram below.  Regardless of the position of
the hash field, the `ObjectReference` of an object usually points at the object header.  In this
way, the header and ordinary fields can be accessed at the same offsets from the `ObjectReference`
regardless of whether or where the hash field has been added.  The starting address of the object,
however, may no longer be the same as he `ObjectReference` in some layout designs.  Therefore, the
VM binding needs to **implement `ObjectModel::ref_to_object_start` and handle the added hash field
correctly**.

```
                                   │ObjectReference                                      
                                   │start of object                                      
                                   ▼                                                     
                                   ┌────────────┬──────────────────────────┐             
No hash field                      │   Header   │ ordinary fields...       │             
                                   └────────────┴──────────────────────────┘             
                                                                                         
                      │start of    │                                                     
                      │object      │ObjectReference                                      
                      ▼            ▼                                                     
                      ┌────────────┬────────────┬──────────────────────────┐             
Hash at the beginning │    Hash    │   Header   │ ordinary fields...       │             
                      └────────────┴────────────┴──────────────────────────┘             
                                                                                         
                                   │ObjectReference                                      
                                   │start of object                                      
                                   ▼                                                     
                                   ┌────────────┬──────────────────────────┬────────────┐
Hash at the end                    │   Header   │ ordinary fields...       │    Hash    │
                                   └────────────┴──────────────────────────┴────────────┘
```

### GC: Copying Objects

MMTk calls the following trait methods implemented by the VM binding during copying GC.

-   For non-delayed-copy collectors (all moving plans except MarkCompact) 
    -   `ObjectModel::copy`
-   For delayed-copy collectors (MarkCompact)
    -   `ObjectModel::copy_to`
    -   `ObjectModel::get_reference_when_copied_to`
    -   `ObjectModel::get_size_when_copied`
    -   `ObjectModel::get_align_when_copied`
    -   `ObjectModel::get_align_offset_when_copied`

When using a non-delayed-copy collector, MMTk calls `ObjectgModel::copy` which is defined as:

```rust
{{#include ../../../../../src/vm/object_model.rs:copy}}
```

The `copy` method should

1.  **Find the state of the `from` object.**  This is done in VM-specific ways, such as inspecting
    header bits.  If it was `Hashed`, it should transition to the `HashedAndMoved` state in the new
    copy.
2.  **Find the size of the `from` object, including the hash field.**  If the `from` object is
    already in the `HashedAndMoved` state, the VM binding must have already inserted a hash field in
    step 3 below.  Make sure the hash field is counted in the object size.
3.  **Allocate the new copy, with a larger size if needed.**  It should call
    `copy_context.alloc_copy(from, new_size, new_align, new_offset, semantics)` to allocate the new
    copy of the object.  When transitioning from `Hashed` to `HashedAndMoved`, the `new_size` should
    be *larger* than the old size in order to accommodate the added hash field.  Otherwise the new
    size should be the same as the old size.
4.  **Adjust the `ObjectReference` if needed.**  If the hash field is inserted in the beginning, the
    offset from the start of the object to the `ObjectReference` may be greater in the new copy.
    Make sure the `ObjectReference` of the new copy is pointing at the right place.  See the
    diagrams in the [Object Layout](#object-layout) section.
5.  **Copy header and ordinary fields.**  Make sure the data is copied to the right offset if the
    hash field is inserted at the beginning.
6.  **Fix the state of the new copy if needed.**  If the old copy is `Hashed`, the new copy shall be
    in the `HashedAndMoved` state.  Set the new copy to the right state by, for example, modifying
    its header bits.
7.  **Write or copy the hash field if needed.**  When transitioning from `Hashed` to
    `HashedAndMoved`, write the old address of the object to the hash field; if the old copy is
    already in the `HashedAndMoved` state, copy the content of the hash field.

When using a delayed-copy collector, the VM binding should do the same things as above, but in
different methods in a slightly different order.  It shall (1) determine the size of the new copy in
`get_size_when_copied`, (2) determine the address of `ObjectReference` in the new copy in
`get_reference_when_copied_to`, and (3) do the actual copying and write the right values to the
header bits and the hash field in `copy_to`.  The reference to the old copy is passed to all of the
three methods as a parameter so that the VM binding can look up the state of the old copy, and
determine the state of the new copy.

### Mutator: Observing the Identity Hash Code

Mutators should get the identity hash code of an object by first finding the state of the object.

-   If `Unhashed`, it should set its state to `Hashed` and use its address as the hash code.
-   If `Hashed`, it should simply use its address as the hash code.
-   If `HashedAndMoved`, it should read the hash code from the added hash field.

Note that the operation of getting the identity hash code may happen concurrently with other mutator
threads and GC worker threads.

Because other mutators can be accessing the header bits of the same object concurrently, the
operation of transitioning the state from `Unhashed` to `Hashed` it should be done *atomically*.
If, as in JikesRVM, the `Unhashed` state is encoded as `00` and the `Hashed` state is encoded as
`01`, this state transition can be done with a single atomic bit-set or fetch-or operation.

There is also a risk if GC can happen concurrently, moving the object and changing its state.  If
copying only happens during stop-the-world (that includes all stop-the-world GC algorithms and
mostly-concurrent GC algorithms that only copy objects during stop-the-world, such as [LXR]), we can
make the computing of identity hash code *atomic with respect to copying GC* by not inserting
[GC-safe points] in the middle of computing identity hash code.  MMTk currently does not have
concurrent copying GC.

[LXR]: https://dl.acm.org/doi/10.1145/3519939.3523440
[GC-safe points]: ../../glossary.md#gc-safe-point

## Alternative Implementation Strategies

Some VMs cannot implement address-based hashing in the same way as JikesRVM due to implementation
details.

If the VM cannot spare two bits in the object header for the hash code state, it can use on-the-side
metadata (bitmap), instead.  It will add a space overhead of two bits per object alignment.  If all
objects are aligned to 8 bytes, it will be a 1/32 (about 3%) space overhead.

If the VM cannot add a hash field to an object when moved, an alternative is using a global table
that maps the address of each object in the `HashedAndMoved` state to its hash code.  This will
require a table lookup every time the VM observes the hash code of an object in the `HashedAndMoved`
state.  This table also needs to be updated when copying GC happens because object may be moved or
dead.  Because the addresses of objects are used as keys, the table may need to be rehashed or
reconstructed if it is implemented as a hash table or a search tree.

Whether the space and time overhead is acceptable depends on the VM implementation as well as the
workload.  For example, if a VM has many hash tables that naively use object addresses as the hash
code, it will need to rehash all such tables when copying GC happens.  Even though maintaining a
global table that maps addresses to hash codes is expensive, implementing address-based hashing this
way can still eliminate the need to rehash all other hash tables at the expense of having to rehash
one global table.  And if very few objects are in the `HashedAndMoved` state, the *average* cost of
computing the identity hash code can still be low.


<!--
vim: tw=100 ts=4 sw=4 sts=4 et
-->
