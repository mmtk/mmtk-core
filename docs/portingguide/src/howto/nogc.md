# Starting a Port: NoGC

We always start a port with NoGC. It is the simplest possible plan: it simply allocates memory and never collects.
Although this appears trivial, depending on the complexity of the runtime and how well factored (or not) its internal GC interfaces are, just getting this working may be a major undertaking.
In the case of V8, the refactoring within V8 required to get a simple NoGC plan working was substantial, touching over 100 files. So it’s a good idea not to underestimate the difficulty of a NoGC port!

At a high level, in order to implement NoGC, we need to handle MMTk initialization, mutator initialization, and memory allocation.

If you're ever stuck at any point, feel free to send a message in the `#Porting` channel of our [Zulip](https://mmtk.zulipchat.com/)!

## Set up
You want to set up the binding repository/directory structure. For the sake of the tutorial guide we assume you have a directory structure similar to the one below. Note that such a directory structure is not a requirement[^1] but a recommendation. We assume you are using some form of version control system (such as `git` or `mercurial`) in this guide.

[^1]: In fact some bindings may not be able to have such a directory structure due to the build tools used by the runtime.

  - `/mmtk` - the MMTk side of the binding. To start with, this can be an almost direct copy of the [Dummy VM binding](https://github.com/mmtk/mmtk-core/tree/master/vmbindings/dummyvm). This is implemented in Rust.
  - `/rt` (rename this to your runtime name) - Runtime-specific code for integrating with MMTk. This should act as a bridge between the generic GC interface offered by the runtime and the MMTk side of the binding. This is implemented in the runtime's implementation language. Often this will be one of C or C++.
  - You can place your runtime repository at any path. For the sake of this guide, we assume you will place the runtime repo as a sibling of the binding repo. You can also clone `mmtk-core` to a local path. Using
    a local repo of `mmtk-core` can be beneficial to your development in case you need to make certain changes to the core (though this is unlikely).
    
Your working directory may look like this (assuming your runtime is named as `X`):
 ```
 Your working directory/
 ├─ mmtk-X/
 │  ├─ X/
 │  └─ mmtk/
 ├─ X/
 └─ mmtk-core/ (optional)
 ```

You may also find it helpful to take inspiration from the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk), particularly for a more complete example of the relevant `Cargo.toml` files. (Note: the use of submodules is no longer recommended).

## Adding a Rust library to the runtime
We recommend learning the ins and outs of your runtime's build system. You should try and add a simple Rust "hello world" library to your runtime's code and build system to investigate how easy it will be to add MMTk. Unfortunately this step is highly dependent on the runtime build system. We recommend taking a look at what other bindings do, but keep in mind that no two runtime build systems are the same even if they are using the same build tools.

In case the build system is too complex and you want get to hacking, a quick and dirty way to add MMTk could be to build a static and/or dynamic binary for MMTk and link it to the language directly, manually building new binaries as necessary, like so:

  1. `cd mmtk-X/mmtk`
  2. `cargo build` to build in debug mode or add `--release` for release mode
  3. Copy the shared or static[^2] library from `target/debug` or `target/release` to your desired location
  
[^2]: You would have to change the `crate-type` in `mmtk-X/mmtk/Cargo.toml` from `cdylib` to `staticlib` to build a static library.

Later, you can edit the runtime build process to build MMTk at the same time automatically.

**Note:** If the runtime you are targeting already links some Rust FFI libraries, then you may notice "multiple definition" linker errors for Rust stdlib functions. Unfortunately this is a current limitation of Rust FFI wherein all symbols are bundled together in the final C lib which will cause multiple definitions errors when two or more Rust FFI libraries are linked together. There is ongoing work to stabilize the Rust package format that would hopefully make it easier in the future. A current workaround would be to use the `-Wl,--allow-multiple-definition` linker flag, but this unfortunately isn't ideal as it increases code sizes. See [here](https://internals.rust-lang.org/t/pre-rfc-stabilize-a-version-of-the-rlib-format/17558) and [here](https://github.com/rust-lang/rust/issues/73632) for more details.

**Note:** It is *highly* recommended to also check-in the generated `Cargo.lock` file into your version control. This is to ensure the same package versions are used when building in the future in order to prevent random breakages and improves reproducibility of the build.

## The `VMBinding` trait
Now let's actually start implementing the binding. Here we take a look at the Rust side of the binding first (i.e. `mmtk-X/mmtk`). What we want to do is implement the [`VMBinding`](https://www.mmtk.io/mmtk-core/public-doc/vm/trait.VMBinding.html) trait.

The `VMBinding` trait is a "meta-trait" (i.e. a trait that encapsulates other traits) that we expect every binding to implement. In essence, it is the contract established between MMTk and the runtime. We discuss each of its seven key traits briefly:

  1. [`ActivePlan`](https://www.mmtk.io/mmtk-core/public-doc/vm/trait.ActivePlan.html): This trait implements functions related to mutators such as how many mutators exist, getting an iterator for all mutators, etc.
  2. [`Collection`](https://www.mmtk.io/mmtk-core/public-doc/vm/trait.Collection.html): This trait implements functions related to garbage collection such as starting and stopping mutators, blocking current mutator thread for GC, etc.
  3. [`ObjectModel`](https://www.mmtk.io/mmtk-core/public-doc/vm/trait.ObjectModel.html): This trait implements the runtime's object model. The object model includes object metadata such as mark-bits, forwarding-bits, etc.; constants regarding assumptions about object addresses; and functions to implement copying objects, querying object sizes, etc. You should ***carefully*** implement and understand this as it is a key trait on which many things depend. We will go into more detail about this trait in the [object model section](#object-model).
  4. [`ReferenceGlue`](https://www.mmtk.io/mmtk-core/public-doc/vm/trait.ReferenceGlue.html): This trait implements runtime-specific finalization and weak reference processing methods. Note that each runtime has its own way of dealing with finalization and reference processing, so this is often one of the trickiest traits to implement.
  5. [`Scanning`](https://www.mmtk.io/mmtk-core/public-doc/vm/trait.Scanning.html): This trait implements object scanning functions such as scanning mutator threads for root pointers, scanning a particular object for reference fields, etc.
  6. [`Edge`](https://www.mmtk.io/mmtk-core/public-doc/vm/edge_shape/trait.Edge.html): This trait implements what an edge in the object graph looks like in the runtime. This is useful as it can abstract over compressed pointer or tagged pointers. If an edge in your runtime is indistinguishable from an arbitrary address, you may set it to the [`Address`](https://www.mmtk.io/mmtk-core/public-doc/util/address/struct.Address.html) type.
  7. [`MemorySlice`](https://www.mmtk.io/mmtk-core/public-doc/vm/edge_shape/trait.MemorySlice.html): This trait implements functions related to memory slices such as arrays. This is mainly used by generational collectors.

For the time-being we can implement all the above traits via `unimplemented!()` stubs. If you are using the Dummy VM binding as a starting point, you will have to edit some of the concrete implementations to `unimplemented!()`.

### Object model

TODO(kunals): Discuss header vs side metadata. Local vs global metadata. ObjectReference <-> Address. Alloc end alignment, etc.

## MMTk initialization
Now we want to actually initialize MMTk.

### Runtime changes
Create a `mmtk.h` header file which exposes the functions required to implement NoGC and `#include` it in the relevant runtime code. You can use the [DummyVM `mmtk.h` header file](https://github.com/mmtk/mmtk-core/blob/master/vmbindings/dummyvm/api/mmtk.h) as an example. Note: It is convention to prefix all MMTk API functions exposed with `mmtk_` in order to avoid name clashes. It is *highly* recommended that you follow this convention.

Having a clean heap API for MMTk to implement makes life easier. Some runtimes may already have a sufficiently clean abstraction such as OpenJDK after the merging of [JEP 304](https://openjdk.org/jeps/304). In (most) other cases, the runtime doesn't provide a clean enough heap API for MMTk to implement. In such cases, it is recommended to create a class (or equivalent) that abstracts allocation and other heap functions like what the [V8](https://chromium.googlesource.com/v8/v8/+/a9976e160f4755990ec065d4b077c9401340c8fb/src/heap/third-party/heap-api.h) and ART bindings do. Ideally these changes are upstreamed like in the case of V8.

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

In order to perform allocations, you will need to know what object alignment the VM expects. VMs often align allocations at word boundaries (e.g. 4- or 8-bytes) as it allows the CPU to access the data faster at execution time. Additionally, the language may use the unused lowest order bits to store flags (e.g. type information), so it is important that MMTk respects these expectations.

  1. Call `mmtk_bind_mutator` on every thread initialization and save the handle in the thread local storage.
  2. Call `mmtk_alloc` and use the stored handle for each thread.

## Miscellaneous implementation steps

### Setting options for MMTk
You can set [options for MMTk](https://www.mmtk.io/mmtk-core/public-doc/util/options/index.html) by using `process` to pass options, or simply by setting environment variables. For example, to use the NoGC plan, you can set the environment variable `MMTK_PLAN=NoGC`. TODO(kunals) talk about environment variables and processing multiple options.

### Runtime-specific steps
TODO(kunals) Describe that certain runtimes may require more than just the above to work. For example the heap iterator in ART.
