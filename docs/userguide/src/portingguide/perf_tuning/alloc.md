# Optimizing Allocation

MMTk provides an allocation function, [`alloc()`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.alloc.html),
and [`post_alloc()`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.post_alloc.html), to allocate a piece of memory, and
finalize the memory as an object. Calling them is sufficient for a functional implementation, and we recommend doing
so in the early development of an MMTk integration. However, as allocation is performance critical, runtimes generally would
optimize to make allocation as fast as possible, in which invoking `alloc()` and `post_alloc()` becomes inadequent.

The following discusses a few design decisions and optimizations related to allocation. The discussion mainly focuses on `alloc()`.
`post_alloc()` works in a similar way, and the discussion can also be applied to `post_alloc()`.
For conrete examples, you can refer to any of our supported bindings, and check the implementation in the bindings.

Note that some of the optimizations need to make assumptions about the MMTk's internal implementation and may make the code less maintainable.
We recommend adding assertions in the binding code to make sure the assumptions are not broken across versions.

## Efficient access to MMTk mutators

MMTk mutators (created by [`bind_mutator()`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.bind_mutator.html)) is a thread local data structure
of type [`Mutator`](https://docs.mmtk.io/api/mmtk/plan/struct.Mutator.html).
MMTk expects the binding to provide efficient access to the mutator structure in their thread local storage (TLS).
Usually one of the following approaches is used to store MMTk mutators.

### Option 1: Storing the pointer

MMTk returns a boxed pointer of `Mutator`. It is simple to store it in the TLS.
This approach does not make any assumption about the intenral of a MMTk `Mutator`. However, it requires one more pointer dereference
to use the mutator, thus has suboptimal performance.

If the implementation language is not Rust,
the binding needs to turn the boxed pointer into a raw pointer before storing it.

```rust
struct TLS {
    ...
    mmtk_mutator: Box<Mutator>,
}

// Bind an MMTk mutator
let mutator = mmtk::memory_manager::bind_mutator(&mmtk, tls_opaque_pointer);
// Store the pointer in TLS.
tls.mmtk_mutator = mutator;

// Allocate
let addr = mmtk::memory_manager::alloc(&tls.mmtk_mutator, ...);
```

### Option 2: Embed the `Mutator` struct

To remove the extra pointer dereference, the binding can embed the `Mutator` type into their TLS type. This saves the extra dereference.

If the implementation language is not Rust, it needs to create a type that has the same layout as `Mutator`. It is recommended to
have an assertion to ensure that the native type has the exact same layout as the Rust type `Mutator`.

```rust
struct TLS {
    ...
    mmtk_mutator: Mutator,
}

// Bind an MMTk Mutator
let mutator = mmtk::memory_manager::bind_mutator(&mmtk, tls_opaque_pointer);
// Store the struct (or use memcpy for non-Rust)
tls.mmtk_mutator = Box::into_inner(mutator);

// Allocator
let addr = mmtk::memory_manager::alloc(&tls.mmtk_mutator, ...);
```

### Option 3: Embed the fastpath struct

The size of `Mutator` is usually a few hundreds of bytes, which could be large for TLS for some langauges.
And it requires to duplicate a native type for the `Mutator` struct if the implementation language is not Rust.
Sometimes it is undesirable to embed the `Mutator` type. One can choose only embed the fastpath struct that is in use.

Unlike the `Mutator` type, the fastpath struct has a C-compatible layout, and it is simple and primitive enough
so it is unlikely to change. For example, MMTk provides [`BumpPointer`](https://docs.mmtk.io/api/mmtk/util/alloc/struct.BumpPointer.html),
which simply includes a `cursor` and a `limit`.

The following example shows how to create a fastpath `BumpPointer`, how to allocate from it in a fast path, and
how to sync values with the mutator struct and call the slow path.

```rust
struct TLS {
    ...
    default_bump_pointer: BumpPointer,
    mmtk_mutator: Box<Mutator>,
}

// Bind an MMTk Mutator
let mutator = mmtk::memory_manager::bind_mutator(&mmtk, tls_opaque_pointer);
// Store the struct (or use memcpy for non-Rust)
tls.mmtk_mutator = mutator;
// Initialize the default allocator -- it only works if the default allocator for the current plan is a bump pointer allocator.
let default_selector = mmtk::memory_manager::get_allocator_mapping(&mmtk, AllocationSemantics::Default);
let default_allocator_in_mutator = tls.mmtk_mutator.allocator_impl::<mmtk::util::alloc::BumpAllocator>(default_selector);
tls.default_bump_pointer = BumpPointer::new(default_allocator_in_mutator.bump_pointer.cursor, default_allocator_in_mutator.bump_pointer.limit);

// Allocate
let new_cursor = tls.default_bump_pointer.cursor + size; // Alignment is ignored.
let addr = if new_cursor < tls.default_bump_pointer.limit {
    // - fastpath: direct allocate from `BumpPointer`.
    let res = tls.default_bump_pointer.cursor;
    tls.default_bump_pointer.cursor = new_cursor;
    res
} else {
    // - slowpath: sync fastpath struct `BumpPointer` with the mutator
    let default_selector = mmtk::memory_manager::get_allocator_mapping(&mmtk, AllocationSemantics::Default);
    let mut default_allocator_in_mutator = tls.mmtk_mutator.allocator_impl_mut::<mmtk::util::alloc::BumpAllocator>(default_selector);
    default_allocator_in_mutator.bump_pointer = tls.default_bump_pointer;
    let res = default_allocator_in_mutator.alloc_slow(...);
    tls.default_bump_pointer = default_allocator_in_mutator.bump_pointer;
    res
};
```

## Avoid resolving the allocator at run time

For a simple and general API of `alloc()`, MMTk requires `AllocationSemantics` as an argument in an allocation request, and resolves it at run-time.
The following is roughly what `alloc()` does internally.

1. Resolving the allocator
    1. Find the `Allocator` for the required `AllocationSemantics`.
    2. Dynamically dispatch the call to [`Allocator::alloc()`](https://docs.mmtk.io/api/mmtk/util/alloc/trait.Allocator.html#tymethod.alloc).
2. `Allocator::alloc()` executes the allocation fast path.
3. If the fastpath fails, it executes the allocation slow path [`Allocator::alloc_slow()`](https://docs.mmtk.io/api/mmtk/util/alloc/trait.Allocator.html#method.alloc_slow).
4. The slow path will further attempt to allocate memory, and may trigger a GC.

Resolving to a specific allocator and doing dynamic dispatch is expensive for an allocation.
With the build-time or JIT-time knowledge on the object that will be allocated, an MMTK binding can possibly skip the first step in the run time.

For example, once MMTK is initialized, a binding can get the memory offset for the default allocator by doing the following:

```rust
let selector = mmtk::memory_manager::get_allocator_mapping(&mmtk, AllocationSemantics::Default);
DEFAULT_ALLOCATOR_BASE_OFFSET = mmtk::plan::Mutator::get_allocator_base_offset(selector);
```

Then when we allocate an object and we know the object should be allocated with the default allocator (`AllocationSemantics::Default`),
we can use the `base_offset` to get the allocator for efficient allocation:

```rust
let mutator_addr = mmtk::util::Address::from_ref(&tls.mmtk_mutator);
let allocator = unsafe { (mutator_addr + DEFAULT_ALLOCATOR_BASE_OFFSET).as_mut_ref::<BumpAllocator>() };
let res = allocator.alloc(...);
```

## Emitting Allocation Sequence in a JIT Compiler

If the language has a JIT compiler, it is generally desirable to generate the code sequence for the allocation fast path, rather
than simply emitting a call instruction to the allocation function. The optimizations we talked above are relevant as well: 1.
the compiler needs to be able to access the mutator, and 2. the compiler needs to be able to resolve to a specific allocator at
JIT time. The actual implementation highly depends on the compiler implementation.

The following are some examples from our bindings (at the time of writing):
* OpenJDK:
  * https://github.com/mmtk/mmtk-openjdk/blob/9ab13ae3ac9c68c5f694cdd527a63ca909e27b15/openjdk/mmtkBarrierSetAssembler_x86.cpp#L38
  * https://github.com/mmtk/mmtk-openjdk/blob/9ab13ae3ac9c68c5f694cdd527a63ca909e27b15/openjdk/mmtkBarrierSetC2.cpp#L45
* JikesRVM: https://github.com/mmtk/mmtk-jikesrvm/blob/fbfb91adafd9e9b3f45bd6a4b32c845a5d48d20b/jikesrvm/rvm/src/org/jikesrvm/mm/mminterface/MMTkMutatorContext.java#L377
* Julia: https://github.com/mmtk/julia/blob/5c406d9bb20d76e2298a6101f171cfac491f651c/src/llvm-final-gc-lowering.cpp#L267
