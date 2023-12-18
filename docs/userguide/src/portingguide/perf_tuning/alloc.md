# Optimizing Allocation

MMTk provides [`alloc()`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.alloc.html)
and [`post_alloc()`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.post_alloc.html), to allocate a piece of memory, and
finalize the memory as an object. Calling them is sufficient for a functional implementation, and we recommend doing
so in the early development of an MMTk integration. However, as allocation is performance critical, runtimes generally would want to
optimize allocation to make it as fast as possible, in which invoking `alloc()` and `post_alloc()` becomes inadequate.

The following discusses a few design decisions and optimizations related to allocation. The discussion mainly focuses on `alloc()`.
`post_alloc()` works in a similar way, and the discussion can also be applied to `post_alloc()`.
For concrete examples, you can refer to any of our supported bindings, and check the implementation in the bindings.

> **Note:** Some of the optimizations need to make assumptions about MMTk's internal implementation and may make the code less maintainable.
We recommend adding assertions in the binding code to make sure the assumptions are not broken across versions.

## Efficient access to MMTk mutators

An MMTk mutator context (created by [`bind_mutator()`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.bind_mutator.html)) is a thread-local data structure
of type [`Mutator`](https://docs.mmtk.io/api/mmtk/plan/struct.Mutator.html).
MMTk expects the binding to provide efficient access to the mutator structure in their thread-local storage (TLS).
Usually one of the following approaches is used to store MMTk mutators.

### Option 1: Storing the pointer

The `Box<Mutator<VM>>` returned from `mmtk::memory_manager::bind_mutator` is actually a pointer to
a `Mutator<VM>` instance allocated in the Rust heap. It is simple to store it in the TLS.
This approach does not make any assumption about the internals of an MMTk `Mutator`. However, it requires an extra pointer dereference
when accessing a value in the mutator. This may sound not too bad, however, this degrades the performance of
a carefully implemented inlined fast-path allocation sequence which is normally just a few (assembly) instructions.
This approach could be a simple start in early development, but we do not recommend it for an efficient implementation.

If the VM is not implemented in Rust,
the binding needs to turn the boxed pointer into a raw pointer before storing it.

```rust
{{#include ../../../../../src/vm/tests/mock_tests/mock_test_doc_mutator_storage.rs:mutator_storage_boxed_pointer}}
```

### Option 2: Embed the `Mutator` struct

To remove the extra pointer dereference, the binding can embed the `Mutator` type into their TLS type. This saves the extra dereference.

If the implementation language is not Rust, the developer needs to create a type that has the same layout as `Mutator`. It is recommended to
have an assertion to ensure that the native type has the exact same layout as the Rust type `Mutator`.

```rust
{{#include ../../../../../src/vm/tests/mock_tests/mock_test_doc_mutator_storage.rs:mutator_storage_embed_mutator_struct}}
```

### Option 3: Embed the fast-path struct

The size of `Mutator` is a few hundreds of bytes, which could be considered too large to store in the TLS in some language implementations.
Embedding `Mutator` also requires to duplicate a native type for the `Mutator` struct if the implementation language is not Rust.
Sometimes it is undesirable to embed the `Mutator` type. One can choose to only embed the fast-path struct that is in use.

Unlike the `Mutator` type, the fast-path struct has a C-compatible layout, and it is simple and primitive enough
so it is unlikely to change. For example, MMTk provides [`BumpPointer`](https://docs.mmtk.io/api/mmtk/util/alloc/struct.BumpPointer.html),
which simply includes a `cursor` and a `limit`.

In the following example, we embed one `BumpPointer` struct in the TLS.
The `BumpPointer` is used in the fast-path, and carefully synchronized with the allocator in the `Mutator` struct in the slow-path. We also need to revoke (i.e. reset) all the cached `BumpPointer` values for *all* mutators if a GC occurs. Currently, we recommend implementing this in the [`resume_mutators`](https://docs.mmtk.io/api/mmtk/vm/trait.Collection.html#tymethod.resume_mutators) API call, however there is work in progress that would make it [an explicit API call instead](https://github.com/mmtk/mmtk-core/issues/1017).

Note that the `allocate_default` closure in the example below assumes the allocation semantics is `AllocationSemantics::Default`
and its selected allocator uses bump-pointer allocation.
Real-world fast-path implementations for high-performance VMs are usually JIT-compiled, inlined, and specialized for the current plan
and allocation site. Hence, the allocation semantics of the concrete allocation site (and therefore the selected allocator) is known to the JIT compiler.

For the sake of simplicity, we only store _one_ `BumpPointer` in the TLS in the example.
In MMTk, each plan has multiple allocators, and the allocation semantics are mapped
to those allocator by the GC plan you choose. So a plan uses multiple allocators, and
depending on how many allocation semantics are used by a binding, the binding may use multiple allocators as well.
In practice, a binding may embed multiple fast-path structs for all the allocators they use if they would like
more efficient allocation.

Also for simplicity, the example assumes the default allocator for the plan in use is a bump pointer allocator.
Many plans in MMTk use bump pointer allocator for their default allocation semantics (`AllocationSemantics::Default`),
which includes (but not limited to) `NoGC`, `SemiSpace`, `Immix`, generational plans, etc.
If a plan does not do bump-pointer allocation, we may still implement fast-paths, but we need to embed different data structures instead of `BumpPointer`.

```rust
{{#include ../../../../../src/vm/tests/mock_tests/mock_test_doc_mutator_storage.rs:mutator_storage_embed_fastpath_struct}}
```

And pseudo-code for how you would reset the `BumpPointer`s for all mutators in `resume_mutators`. Note that these mutators are the runtime's actual mutator threads (i.e. where the cached bump pointers are stored) and are different from MMTk's `Mutator` struct.
```rust
impl Collection<RtName> for RtNameCollection {
  ...
  fn resume_mutators(tls: VMWorkerThread) {
    // Reset the cached bump pointers of each mutator (setting both cursor and limit to 0) after a GC to
    // ensure that the VM sees a cohesive state
    for mutator in mutators {
      mutator.storage.default_bump_pointer = BumpPointer::default();
    }
    // Actually resume all the mutators now
    ...
  }
  ...
}
```

## Avoid resolving the allocator at run time

For a simple and general API of `alloc()`, MMTk requires `AllocationSemantics` as an argument in an allocation request, and resolves it at run-time.
The following is roughly what `alloc()` does internally.

1. Resolving the allocator
    1. Find the `Allocator` for the required `AllocationSemantics`. It is defined by the plan in use.
    2. Dynamically dispatch the call to [`Allocator::alloc()`](https://docs.mmtk.io/api/mmtk/util/alloc/trait.Allocator.html#tymethod.alloc).
2. `Allocator::alloc()` executes the allocation fast-path.
3. If the fast-path fails, it executes the allocation slow-path [`Allocator::alloc_slow()`](https://docs.mmtk.io/api/mmtk/util/alloc/trait.Allocator.html#method.alloc_slow).
4. The slow-path will further attempt to allocate memory, and may trigger a GC.

Resolving to a specific allocator and doing dynamic dispatch is expensive for an allocation.
With the build-time or JIT-time knowledge about the object that will be allocated, an MMTk binding can possibly skip the first step in the run time.

If you implement an efficient fast-path allocation in the binding side (like the Option 3 above, and [generating allocation code in a JIT](#emitting-allocation-sequence-in-a-jit-compiler)),
that naturally avoids this problem. If you do not want to implement the fast-path allocation, the following is another example of how to avoid resolving the allocator.

Once MMTk is initialized, a binding can get the memory offset for the default allocator, and save it somewhere. When we know an object should be allocated
with the default allocation semantics, we can use the offset to get a reference to the actual allocator (with unsafe code), and allocate with the allocator.

```rust
{{#include ../../../../../src/vm/tests/mock_tests/mock_test_doc_avoid_resolving_allocator.rs:avoid_resolving_allocator}}
```

## Emitting Allocation Sequence in a JIT Compiler

If the language has a JIT compiler, it is generally desirable to generate the code sequence for the allocation fast-path, rather
than simply emitting a call instruction to the allocation function. The optimizations we talked above are relevant as well: (i)
the compiler needs to be able to access the mutator, and (ii) the compiler needs to be able to resolve to a specific allocator at
JIT time. The actual implementation highly depends on the compiler implementation.

The following are some examples from our bindings (at the time of writing):
* OpenJDK:
  * [Example 1 (C1 compiler)](https://github.com/mmtk/mmtk-openjdk/blob/9ab13ae3ac9c68c5f694cdd527a63ca909e27b15/openjdk/mmtkBarrierSetAssembler_x86.cpp#L38)
  * [Example 2 (C2 compiler)](https://github.com/mmtk/mmtk-openjdk/blob/9ab13ae3ac9c68c5f694cdd527a63ca909e27b15/openjdk/mmtkBarrierSetC2.cpp#L45)
* [JikesRVM](https://github.com/mmtk/mmtk-jikesrvm/blob/fbfb91adafd9e9b3f45bd6a4b32c845a5d48d20b/jikesrvm/rvm/src/org/jikesrvm/mm/mminterface/MMTkMutatorContext.java#L377)
* [Julia](https://github.com/mmtk/julia/blob/5c406d9bb20d76e2298a6101f171cfac491f651c/src/llvm-final-gc-lowering.cpp#L267)
