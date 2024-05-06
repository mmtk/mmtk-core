# Starting a Port: NoGC

We always start a port with NoGC. It is the simplest possible plan: it simply allocates memory and never collects.
Although this appears trivial, depending on the complexity of the runtime and how well factored (or not) its internal GC interfaces are, just getting this working may be a major undertaking.
In the case of V8, the refactoring within V8 required to get a simple NoGC plan working was substantial, touching over 100 files. So it’s a good idea not to underestimate the difficulty of a NoGC port!

At a high level, in order to implement NoGC, we need to handle MMTk initialization, mutator initialization, and memory allocation.

If you're ever stuck at any point, feel free to send a message in the `#Porting` channel of our [Zulip](https://mmtk.zulipchat.com/)!

## Set up
You want to set up the binding repository/directory structure before starting the port. For the sake of the tutorial guide we assume you have a directory structure similar to the one below. Note that such a directory structure is not a requirement[^1] but a recommendation. We assume you are using some form of version control system (such as `git` or `mercurial`) in this guide.

[^1]: In fact some bindings may not be able to have such a directory structure due to the build tools used by the runtime.

  - `mmtk-X/mmtk`: The MMTk side of the binding. This includes the implementation of [the `VMBinding` trait](https://docs.mmtk.io/api/mmtk/vm/trait.VMBinding.html),
    and any necessary Rust code to integrate MMTk with the VM code (e.g. exposing MMTk functions to native, allowing up-calls from the MMTk binding to the runtime, etc).
    To start with, you can take a look at one of our officially maintained language bindings as an example: [OpenJDK](https://github.com/mmtk/mmtk-openjdk/tree/master/mmtk),
    [JikesRVM](https://github.com/mmtk/mmtk-jikesrvm/tree/master/mmtk), [V8](https://github.com/mmtk/mmtk-v8/tree/master/mmtk), [Julia](https://github.com/mmtk/mmtk-julia/tree/master/mmtk),
    [V8](https://github.com/mmtk/mmtk-v8/tree/master/mmtk).
  - `mmtk-X/X`: Runtime-specific code for integrating with MMTk. This should act as a bridge between the generic GC interface offered by the runtime and the MMTk side of the binding. This is implemented in the runtime's implementation language. Often this will be one of C or C++.
  - You can place your runtime repository at any path. For the sake of this guide, we assume you will place the runtime repo as a sibling of the binding repo. You can also clone `mmtk-core` to a local path. Using a local repo of `mmtk-core` can be beneficial to your development in case you need to make certain changes to the core (though this is unlikely).

Your working directory may look like this (assuming your runtime is named as `X`):
 ```
 Your working directory/
 ├─ mmtk-X/
 │  ├─ X/
 │  └─ mmtk/
 ├─ X/
 └─ mmtk-core/ (optional)
 ```

You may also find it helpful to take inspiration from the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk), particularly for a more complete example of the relevant `Cargo.toml` files.

For this guide, we will assume your runtime is implemented in C or C++ as they are the most common implementation languages. However note that your runtime does not *need* to be implemented in C/C++ to work with MMTk.

## Adding a Rust library to the runtime
We recommend learning the ins and outs of your runtime's build system. You should try and add a simple Rust "hello world" library to your runtime's code and build system to investigate how easy it will be to add MMTk. Unfortunately this step is highly dependent on the runtime build system. We recommend taking a look at what other bindings do, but keep in mind that no two runtime build systems are the same even if they are using the same build tools.

In case the build system is too complex and you want get to hacking, a quick and dirty way to add MMTk could be to build a static and/or dynamic binary for MMTk and link it to the runtime directly, manually building new binaries as necessary, like so:

  1. `cd mmtk-X/mmtk`
  2. `cargo build` to build in debug mode or add `--release` for release mode
  3. Copy the shared or static[^2] library from `target/debug` or `target/release` to your desired location

[^2]: You would have to change the `crate-type` in `mmtk-X/mmtk/Cargo.toml` from `cdylib` to `staticlib` to build a static library.

Later, you can edit the runtime build process to build MMTk at the same time automatically.

**Note:** If the runtime you are targeting already links some Rust FFI libraries, then you may notice "multiple definition" linker errors for Rust stdlib functions. Unfortunately this is a current limitation of Rust FFI wherein all symbols are bundled together in the final C lib which will cause multiple definitions errors when two or more Rust FFI libraries are linked together. There is ongoing work to stabilize the Rust package format that would hopefully make it easier in the future. A current workaround would be to use the `-Wl,--allow-multiple-definition` linker flag, but this unfortunately isn't ideal as it increases code sizes. See [here](https://internals.rust-lang.org/t/pre-rfc-stabilize-a-version-of-the-rlib-format/17558) and [here](https://github.com/rust-lang/rust/issues/73632) for more details.

**Note:** It is *highly* recommended to also check-in the generated `Cargo.lock` file into your version control. This improves the reproducibility of the build and ensures the same package versions are used when building in the future in order to prevent random breakages.

We recommend using the `debug` build when doing development work as it has helpful logging statements and assertions that will make catching bugs in your implementation easier.

## The `VMBinding` trait
Now let's actually start implementing the binding. Here we take a look at the Rust side of the binding first (i.e. `mmtk-X/mmtk`). What we want to do is implement the [`VMBinding`](https://docs.mmtk.io/api/mmtk/vm/trait.VMBinding.html) trait.

The `VMBinding` trait is a "meta-trait" (i.e. a trait that encapsulates other traits) that we expect every binding to implement. In essence, it is the contract established between MMTk and the runtime. We discuss each of its seven key traits briefly:

  1. [`ActivePlan`](https://docs.mmtk.io/api/mmtk/vm/trait.ActivePlan.html): This trait implements functions related to mutators such as how many mutators exist, getting an iterator for all mutators, etc.
  2. [`Collection`](https://docs.mmtk.io/api/mmtk/vm/trait.Collection.html): This trait implements functions related to garbage collection such as starting and stopping mutators, blocking current mutator thread for GC, etc.
  3. [`ObjectModel`](https://docs.mmtk.io/api/mmtk/vm/trait.ObjectModel.html): This trait implements the runtime's object model. The object model includes object metadata such as mark-bits, forwarding-bits, etc.; constants regarding assumptions about object addresses; and functions to implement copying objects, querying object sizes, etc. You should ***carefully*** implement and understand this as it is a key trait on which many things depend. We will go into more detail about this trait in the [object model section](#object-model).
  4. [`ReferenceGlue`](https://docs.mmtk.io/api/mmtk/vm/trait.ReferenceGlue.html): This trait implements runtime-specific finalization and weak reference processing methods. Note that each runtime has its own way of dealing with finalization and reference processing, so this is often one of the trickiest traits to implement.
  5. [`Scanning`](https://docs.mmtk.io/api/mmtk/vm/trait.Scanning.html): This trait implements object scanning functions such as scanning mutator threads for root pointers, scanning a particular object for reference fields, etc.
  6. [`Edge`](https://docs.mmtk.io/api/mmtk/vm/edge_shape/trait.Edge.html): This trait implements what an edge in the object graph looks like in the runtime. This is useful as it can abstract over compressed or tagged pointers. If an edge in your runtime is indistinguishable from an arbitrary address, you may set it to the [`Address`](https://docs.mmtk.io/api/mmtk/util/address/struct.Address.html) type.
  7. [`MemorySlice`](https://docs.mmtk.io/api/mmtk/vm/edge_shape/trait.MemorySlice.html): This trait implements functions related to memory slices such as arrays. This is mainly used by generational collectors.

For the time-being we can implement all the above traits via `unimplemented!()` stubs. If you are using the Dummy VM binding as a starting point, you will have to edit some of the concrete implementations to `unimplemented!()`. Note that you should change the type that implements `VMBinding` from `DummyVM` to an appropriately named type for your runtime. For example, the OpenJDK binding defines the zero-struct [`OpenJDK`](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/lib.rs#L139-L162) which implements the `VMBinding` trait.

### Object model

The `ObjectModel` trait is a fundamental trait describing the layout of an object to MMTk. This is important as MMTk's core doesn't know of how objects look like internally as each runtime will be different. There are certain key aspects you need to be aware of while implementing the `ObjectModel` trait. We discuss them in this section.

#### Header vs Side metadata

Per-object metadata can live in one of two places: in the object header or in a separate space used just for metadata. Each one has its pros and cons.

Header metadata sits in close proximity to the actual object address but it is not easy to perform bulk operations. On the other hand, side metadata sits in a dedicated metadata space where each possible object address is assigned some metadata. This makes performing bulk operations easy and does not require stealing bits from the object header (there may in fact be no bits to steal for certain runtimes), but can result in large heap sizes given the metadata space is counted as part of the heap.

The choice of metadata location depends on the runtime and its object model and header layout. For example the JikesRVM runtime reserved extra space at the start of each object for GC-related metadata. Such space may not be available in your runtime. In such cases you can use side metadata to reserve per-object metadata.

#### Local vs Global metadata

MMTk uses multiple GC policies and each policy may use a different set of object metadata from each other. A moving policy, for example, may require extra metadata (in comparison to a non-moving policy) to store the forwarding bits and forwarding pointer. Such a metadata, which is local to a policy, is referred to as "local" metadata.

However, in certain cases, we may need to have metadata globally for the entire heap space. The classic example is the valid-object bit metadata which tells us if an arbitrary address is allocated/managed by MMTk. Such a metadata, which spans multiple policies, is referred to as "global" metadata.

For example, the *Forwarding bits and pointer* metadata is a local metadata used by copying policies to store forwarding bits (2-bits) and forwarding pointers (word size). Often runtimes require word-aligned addresses which means we can use the last two bits in the object header (due to alignment) and the entire object header to store the forwarding bits and pointer respectively. This metadata is almost always in the header.

We recommend going through the [list of metadata specifications](https://docs.mmtk.io/api/mmtk/vm/trait.ObjectModel.html#required-associated-consts) that are defined by MMTk. You should set them to locations that are appropriate for your runtime.

#### `ObjectReference` vs `Address`

A key principle in MMTk is the distinction between [`ObjectReference`](https://docs.mmtk.io/api/mmtk/util/address/struct.ObjectReference.html) and [`Address`](https://docs.mmtk.io/api/mmtk/util/address/struct.Address.html). The idea is that very few operations are allowed on an `ObjectReference`. For example, MMTk does not allow address arithmetic on `ObjectReference`s. This allows us to preserve memory-safety, only performing unsafe operations when required, and gives us a cleaner and more flexible abstraction to work with as it can allow object handles or offsets etc. `Address`, on the other hand, represents an arbitrary machine address.

You might be interested in reading the *Demystifying Magic: High-level Low-level Programming* paper[^3] which describes the above in more detail.

[^3]: https://users.cecs.anu.edu.au/~steveb/pubs/papers/vmmagic-vee-2009.pdf

#### Miscellaneous configuration options

There are many constants in the `ObjectModel` trait that can be overridden in your binding in order to meet your runtime's requirements. For example, the `OBJECT_REF_OFFSET_LOWER_BOUND` constant which defines the minimum offset from allocation result start (i.e. the address that MMTk will return to the runtime) and the actual start of the object, i.e. the `ObjectReference`. In other words, the constant represents the minimum offset from the allocation result start such that the following invariant always holds:

    OBJECT_REFERENCE >= ALLOCATION_RESULT_START + OFFSET

We recommend going through the [list of constants in the documentation](https://docs.mmtk.io/api/mmtk/vm/trait.ObjectModel.html) and seeing if the default values suit your runtime's semantics, changing them if required.

## MMTk initialization
Now that we have most of the boilerplate set up, the next step is to initialize MMTk so that we can start allocating objects.

### Runtime-side changes
Create a `mmtk.h` header file in the runtime folder of the binding (i.e. `mmtk-X/X`) which exposes the functions required to implement NoGC and `#include` it in the relevant runtime code. You can use the [example `mmtk.h` header file](https://github.com/mmtk/mmtk-core/blob/master/docs/header/mmtk.h) as an example.

**Note:** It is convention to prefix all MMTk API functions exposed with `mmtk_` in order to avoid name clashes. It is *highly* recommended that you follow this convention.

Having a clean heap API for MMTk to implement makes life easier. Some runtimes may already have a sufficiently clean abstraction such as OpenJDK after the merging of [JEP 304](https://openjdk.org/jeps/304). In (most) other cases, the runtime doesn't provide a clean enough heap API for MMTk to implement. In such cases, it is recommended to create a class (or equivalent) that abstracts allocation and other heap functions like what the [V8](https://chromium.googlesource.com/v8/v8/+/a9976e160f4755990ec065d4b077c9401340c8fb/src/heap/third-party/heap-api.h) and ART bindings do. This allows making minimal changes to the actual runtime and having a concrete implementation of the exposed heap API in the binding, reducing MMTk-specific code in the runtime. Ideally these changes are upstreamed like in the case of V8.

It is also recommended that any change you do in the runtime be guarded by build-time flags as it helps in maintaining a clean port.

At this step, your `mmtk.h` file may look something like this:
```C
#ifndef MMTK_H
#define MMTK_H

#include <stddef.h>
#include <sys/types.h>

// The extern "C" is only required if the runtime
// implementation language is C++
extern "C" {

// An arbitrary address
typedef void* Address;
// MmtkMutator should be an opaque pointer for the VM
typedef void* MmtkMutator;
// An opaque pointer to a VMThread
typedef void* VMThread;

/**
 * Initialize MMTk instance
 */
void mmtk_init();

/**
 * Set the heap size
 *
 * @param min minimum heap size
 * @param max maximum heap size
 */
void mmtk_set_heap_size(size_t min, size_t max);

} // extern "C"

#endif // MMTK_H
```

Now we can initialize MMTk in the runtime. Note that MMTk should ideally be initialized around when the default heap of the runtime is initialized. You will have to figure out where is the best location to initialize MMTk in your runtime.

Initializing MMTk requires two steps. First, we set the heap size by calling `mmtk_set_heap_size` with the initial heap size and the maximum heap size. Then, we initialize MMTk by calling `mmtk_init`. In the future, you may wish to make the heap size configurable via a command line argument or environment variable (See [setting options for MMTk](#setting-options-for-mmtk)).

<!-- You may have noticed the `mmtk_initialize_collection` function defined above in the `mmtk.h` file. This function is called after the runtime has completely set up including (but not limited to) its thread system. This function will spawn GC threads and allow MMTk to collect objects. For the time-being we can ignore calling this function as NoGC does not collect objects so does not require calling `mmtk_initialize_collection`. -->

### MMTk-side changes
On the Rust side of the binding, we want to implement the two functions exposed by the `mmtk.h` file above. We use an [`MMTKBuilder`](https://docs.mmtk.io/api/mmtk/struct.MMTKBuilder.html) instance to actually create our concrete [`MMTK`](https://docs.mmtk.io/api/mmtk/struct.MMTK.html) instance. We recommend following the paradigm used by all our bindings wherein we have a `static` single `MMTK` instance and an `MMTKBuilder` instance that we can use to set relevant options. See the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/lib.rs#L169-L178) for an example.

**Note:** MMTk currently assumes that there is only one `MMTK` instance in your runtime process. Multiple `MMTK` instances are currently not supported.

The `mmtk_set_heap_size` function is fairly straightforward. We recommend using the implementation in the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/api.rs#L94-L104). The `mmtk_init` function is straightforward as well. It should simply manually initialize the `MMTK` `static` variable using `lazy_static`, like [here](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/api.rs#L83-L86) in the OpenJDK binding.

By this point, you should have MMTk initialized. If you are using a debug build (which is recommended) and have logging turned on a message similar to below would be printed out:

```
[...]
[INFO  mmtk::memory_manager] Initialized MMTk with NoGC (FixedHeapSize(10485760))
[...]
```

## Binding mutator threads to MMTk

For MMTk to allocate objects, it needs to be aware of mutator threads. MMTk only allows mutator threads to allocate objects. We do this by "binding" a mutator thread to MMTk when it is initialized in the runtime.

### Runtime-side changes

Add the following function to the `mmtk.h` file:

```C
[...]

/**
 * Bind a mutator thread in MMTk
 *
 * @param tls pointer to mutator thread
 * @return an instance of an MMTk mutator
 */
MmtkMutator mmtk_bind_mutator(VMThread tls);

[...]
```

The `mmtk_bind_mutator` function takes in an opaque pointer representing an instance of the runtime's mutator thread and returns an opaque pointer to a [`Mutator`](https://docs.mmtk.io/api/mmtk/plan/struct.Mutator.html) instance back to the runtime. The runtime ***must*** store this pointer somewhere, preferably in its runtime thread local storage implementation, as MMTk requires a `Mutator` instance to allocate and perform other actions.

The placement of the `mmtk_bind_mutator` call in the runtime depends on the runtime's implementation of its thread system. It is recommended to call `mmtk_bind_mutator` when the runtime initializes the thread local storage of a newly created thread. This ensures that the thread can allocate from MMTk immediately after initialization.

### MMTk-side changes

The Rust side of the binding should simply defer the actual implementation to [`mmtk::memory_manager::bind_mutator`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.bind_mutator.html). See the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/api.rs#L106-L109) for an example.

## Allocation
Now we can finally implement the allocation functions.

### Runtime-side changes
Add the following two functions to the `mmtk.h` file:

```C
[...]

/**
 * Allocate an object
 *
 * @param mutator the mutator instance that is requesting the allocation
 * @param size the size of the requested object
 * @param align the alignment requirement for the object
 * @param offset the allocation offset for the object
 * @param allocator the allocation semantics to use for the allocation
 * @return the address of the newly allocated object
 */
void *mmtk_alloc(MmtkMutator mutator, size_t size, size_t align,
        ssize_t offset, int allocator);

/**
 * Set relevant object metadata
 *
 * @param mutator the mutator instance that is requesting the allocation
 * @param object the returned address of the allocated object
 * @param size the size of the allocated object
 * @param allocator the allocation semantics to use for the allocation
 */
void mmtk_post_alloc(MmtkMutator mutator, void* object, size_t size, int allocator);

[...]
```

In order to perform allocations, you will need to know what object alignment the runtime expects. Runtimes often align allocations at word boundaries (i.e. 4- or 8-bytes) as it allows the CPU to access the data faster at execution time. Additionally, the runtime may use the unused lowest order bits to store flags (e.g. type information), so it is important that MMTk respects these expectations. Once you have figured out the alignment requirements for your runtime, you should update the [`MIN_ALIGNMENT`](https://docs.mmtk.io/api/mmtk/vm/trait.VMBinding.html#associatedconstant.MIN_ALIGNMENT) constant in `VMBinding` to the correct value.

Now that MMTk is aware of each mutator thread, you have to change the runtime's allocation functions to call into MMTk to allocate using `mmtk_alloc` and set object metadata using `mmtk_post_alloc`. Note that there may be multiple allocation functions in the runtime so make sure that you edit them all!

You should use the saved `Mutator` pointer as the first parameter, the requested object size as the next parameter, and any alignment requirements the runtimes has as the third parameter.

If your runtime requires a non-zero allocation offset (i.e. the alignment requirements are for the offset address, not the returned address) then you have to provide the required value as the fourth parameter. Note that you ***must*** also update the [`USE_ALLOCATION_OFFSET`](https://docs.mmtk.io/api/mmtk/vm/trait.VMBinding.html#associatedconstant.USE_ALLOCATION_OFFSET) constant in the `VMBinding` implementation if your runtime requires a non-zero allocation offset.

For the time-being, you can ignore the `allocator` parameter in both these functions and always pass a value of `0` which means MMTk will pick the default allocator for your collector (a bump pointer allocator in the case of NoGC).

Finally, you need to call `mmtk_post_alloc` with the object address returned from the previous `mmtk_alloc` call in order to initialize object metadata.

**Note:** Currently MMTk assumes object sizes are multiples of the `MIN_ALIGNMENT`. If you encounter errors with alignment, a simple workaround would be to align the requested object size up to the `MIN_ALIGNMENT`. See [here](https://github.com/mmtk/mmtk-core/issues/730) for the tracking issue to fix this bug.

### MMTk-side changes

The Rust side of the binding should simply defer the actual implementation to [`mmtk::memory_manager::alloc`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.alloc.html) and [`mmtk::memory_manager::post_alloc`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.post_alloc.html) respectively. See the [OpenJDK](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/api.rs#L125-L136) [binding](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/api.rs#L151-L161) for an example.

Congratulations! At this point, you hopefully have object allocation working and can run simple programs with your runtime using MMTk!

## Miscellaneous implementation steps

### Setting options for MMTk

The preferred method of setting [options for MMTk](https://docs.mmtk.io/api/mmtk/util/options/index.html) is by setting them via the `MMTKBuilder` instance. See [here](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/api.rs#L79) for an example in the OpenJDK binding.

The [`process`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.process.html) function can also be used to pass options. You may want to set multiple options at the same time. In such a case you can use the [`process_bulk`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.process_bulk.html) function.

MMTk also supports setting options via environment variables. This is generally only recommended at early stages of the porting process in order for quick development. For example, to use the NoGC plan, you can set the environment variable `MMTK_PLAN=NoGC`.

A full list of available options that you can set can be found [here](https://docs.mmtk.io/api/mmtk/util/options/struct.Options.html).

### Runtime-specific steps

Often it is the case that the above changes are not enough to allow a runtime to work with MMTk. For example, for the ART binding, the runtime required that all inflated locks be deflated prior to writing the boot image. In order to fix this, we had to implement a heap visitor that visited each allocated object and checked if it had inflated locks, deflating them if they were.

Unfortunately there is no real magic bullet here. If you come across a runtime-specific idiosyncrasy (and you almost certainly will), you will have to understand what the underlying bug is and either fix or work around it.

If you have any confusions or questions, please free to reach us on our [Zulip](https://mmtk.zulipchat.com/)! We would be glad to help.
