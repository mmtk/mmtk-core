# NoGC

We always start a port with NoGC. It is the simplest possible plan, it simply allocates memory and never collects.
Although this appears trivial, depending on the complexity of the runtime and how well factored (or not) its internal GC interfaces are, just getting this working may be a major undertaking.
In the case of V8, the refactoring within V8 required to get a simple NoGC plan working was substantial, touching over 100 files. So it’s a good idea not to underestimate the difficulty of a NoGC port!

At a high level, in order to implement NoGC, we need to handle MMTk initialization, mutator initialization, and memory allocation.

## Set up
You want to set up the binding repository/directory structure now. 

  - `/mmtk` - the MMTk side of the binding. To start, this can be an almost direct copy of the [Dummy VM binding](https://github.com/mmtk/mmtk-core/tree/master/vmbindings/dummyvm).
  - `/vm` (rename this to your VM name) - VM-specific code for integrating with MMTk. This should act as a bridge between the generic GC interface offered by the VM and the MMTk side of the binding.
  - You can place your VM repository at any path. For clarity, we assume you will place the VM repo as a sibling of the binding repo. You can also clone `mmtk-core` to a local path, and using
    a local repo of `mmtk-core` will help a lot in your development. So your working directory would look like this (assuming your VM is named as `X`):
    ```
    Your working directory/
    ├─ mmtk-X/
    │  ├─ X/
    │  └─ mmtk/
    ├─ X/
    └─ mmtk-core/ (optional)
    ```
  - You may also find it helpful to take inspiration from the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk), particularly for a more complete example of the relevant `Cargo.toml` files. (Note: the use of submodules is no longer recommended).

## Adding a Rust library to the runtime
It may be easier to simply build a static and/or dynamic binary for MMTk and link it to the language directly, manually building new binaries as necessary. 

  1. `cd mmtk-X/mmtk`
  2. `cargo build` to build in debug mode or add `--release` for release mode
  3. Copy the shared or static library from `target/debug` or `target/release` to your desired location

If the runtime you are targeting already links some Rust FFI libraries, then you may notice "multiple definition" linker errors for Rust stdlib functions. Unfortunately this is a current limitation of Rust FFI wherein all symbols are bundled together in the final C lib which will cause multiple definitions errors when two or more Rust FFI libraries are linked together. There is ongoing work to stabilize the Rust package format that would hopefully make it easier in the future. A current workaround would be to use the `-Wl,--allow-multiple-definition` linker flag, but this unfortunately isn't ideal as it increases code sizes. See more [here](https://internals.rust-lang.org/t/pre-rfc-stabilize-a-version-of-the-rlib-format/17558) and [here](https://github.com/rust-lang/rust/issues/73632).

Later, you can edit the language build process to build MMTk at the same time automatically.

## The `VMBinding` trait
TODO(kunals) Discuss the `VMBinding` trait and object model/metadata.

## MMTk initialization
### Runtime changes
Create a `mmtk.h` header file which exposes the functions required to implement NoGC and `#include` it in the relevant runtime code. You can use the [DummyVM `mmtk.h` header file](https://github.com/mmtk/mmtk-core/blob/master/vmbindings/dummyvm/api/mmtk.h) as an example. Note: It is convention to prefix all MMTk API functions exposed with `mmtk_` in order to avoid name clashes. It is *highly recommended* that you follow this convention.

Having a clean heap API for MMTk to implement makes life easier. Some runtimes may already have a sufficiently clean abstraction such as OpenJDK after the merging of [JEP 304](https://openjdk.org/jeps/304). In (most) other cases, the runtime doesn't provide a clean enough heap API for MMTk to implement. In such cases, it is recommended to create a class (or equivalent) that abstracts allocation and other heap functions such as the [V8](https://chromium.googlesource.com/v8/v8/+/a9976e160f4755990ec065d4b077c9401340c8fb/src/heap/third-party/heap-api.h) and ART bindings. Ideally these changes are upstreamed, like in the case of V8.

It is also recommended that any change you do in the runtime be guarded by some build-time flags as it helps in maintaining a clean port.

At this step your `mmtk.h` file may look something like this:
```C
#ifndef MMTK_H
#define MMTK_H

#include <stddef.h>
#include <sys/types.h>

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
 * Initialize collection for MMTk
 *
 * @param tls reference to the calling VMThread
 */
void mmtk_initialize_collection(VMThread tls);

/**
 * Set the heap size
 *
 * @param min minimum heap size
 * @param max maximum heap size
 */
void mmtk_set_heap_size(size_t min, size_t max);

/**
 * Get the heap start
 *
 * @return the starting heap address
 */
Address mmtk_get_heap_start();

/**
 * Get the heap end
 *
 * @return the ending heap address
 */
Address mmtk_get_heap_end();

/**
 * Allocation
 *
 * Functions that interact with the mutator and are responsible for allocation
 */

/**
 * Bind a mutator thread in MMTk
 *
 * @param tls pointer to mutator thread
 * @return an instance of an MMTk mutator
 */
MmtkMutator mmtk_bind_mutator(VMThread tls);

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

} // extern "C"

#endif // MMTK_H
```

We now want to initialize MMTK. This has two parts: inserting calls in the runtime to initialize MMTk and actually initializing the MMTk instance in the Rust part of the binding. Most of the work we have to do in this step is in the Rust part of the binding.

Initialize the heap size by calling `mmtk_set_heap_size` with the initial heap size and the maximum heap size. Then initialize MMTk by calling `mmtk_init`. In the future, you may wish to make the heap size configurable via a command line argument or environment variable.

### Rust binding
On the Rust side of the binding, we first want to define a type that will implement the [`VMBinding`](https://www.mmtk.io/mmtk-core/public-doc/vm/trait.VMBinding.html) trait. If you are using the `DummyVM` binding as a starting point, you should rename the `DummyVM` type to your the name of your runtime. For example for the OpenJDK binding, we define the zero-struct [`OpenJDK`](https://github.com/mmtk/mmtk-openjdk/blob/54a249e877e1cbea147a71aafaafb8583f33843d/mmtk/src/lib.rs#L139-L162) which implements the `VMBinding` trait.

## Binding mutator threads to MMTk
Create a MMTk mutator instance using `mmtk_bind_mutator`.

## Allocation
Replace allocation calls with `mmtk_alloc`. The MMTk handle is the return value of the `mmtk_bind_mutator` call.

In order to perform allocations, you will need to know what object alignment the VM expects. VMs often align allocations at word boundaries (e.g. 4 or 8 bytes) as it allows the CPU to access the data faster at runtime. Additionally, the language may use the unused lowest order bits to store flags (e.g. type information), so it is important that MMTk respects these expectations.

    1. Call `mmtk_bind_mutator` on every thread initialization and save the handle in the thread local storage.
    2. Call `mmtk_alloc` and use the stored handle for each thread.

## Miscellaneous implementation steps

### Setting options for MMTk
You can set [options for MMTk](https://www.mmtk.io/mmtk-core/public-doc/util/options/index.html) by using `process` to pass options, or simply by setting environment variables. For example, to use the NoGC plan, you can set the environment variable `MMTK_PLAN=NoGC`. TODO(kunals) talk about environment variables and processing multiple options.

### Runtime-specific steps
TODO(kunals) Describe that certain runtimes may require more than just the above to work. For example the heap iterator in ART.
