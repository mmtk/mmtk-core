# Collecting Garbage: Getting Started with Integrating MMTk

Your choice of the next GC plan to implement depends on your situation.
If you’re developing a new VM from scratch, or if you are intimately familiar with the internals of your target VM, then implementing a SemiSpace collector is probably the best course of action.
Although the GC itself is rather simplistic, it stresses many of the key components of the MMTk <-> VM binding that will be required for later (and more powerful) GCs.
In particular, since it always moves objects, it is an excellent stress test.
Otherwise, a non-moving GC like MarkSweep or a non-moving Immix implementation would work better.

We note that most of the API you need to implement between the moving and non-moving GC will be the same (with moving GCs having to implement a few extra APIs), so regardless of what you choose, the steps in this guide will be applicable.
For this guide, we start by integrating a non-moving Immix implementation and then add support for moving objects.
In order to use a non-moving Immix implementation, enable the ["immix_non_moving" feature of mmtk-core](TODO(kunals)).
We also recommend turning the ["immix_zero_on_release" feature](TODO(kunals)) on for debugging.

Like with the [NoGC guide](./nogc.md), "Runtime-side changes" mean any changes you have to make to your runtime or the part of the MMTk binding interfacing with the runtime; and "MMTk-side changes" mean any changes you have to make to the part of the MMTk binding interfacing with MMTk core.

## Initializing and Enabling Collection

In the NoGC port, we actually skipped over initializing and enabling garbage collection as we were only concerned with allocating objects. This is required as MMTk spawns GC threads when you enable collection.
This is a separate step as it is often the case that the threading subsystem of a runtime has not been fully set up when the `MMTK` instance is created.

<!-- You may have noticed the `mmtk_initialize_collection` function defined above in the `mmtk.h` file. This function is called after the runtime has completely set up including (but not limited to) its thread system. This function will spawn GC threads and allow MMTk to collect objects. For the time-being we can ignore calling this function as NoGC does not collect objects so does not require calling `mmtk_initialize_collection`. -->

### Runtime-side changes

Add the following function to the `mmtk.h` file:

```C
[...]

/**
 * Initialize collection for MMTk
 *
 * @param tls reference to the calling VMThread
 */
void mmtk_initialize_collection(VMThread tls);

[...]
```

You should call this function after the threading subsystem of your runtime has initialized and allows new threads to be spawned.
You can pass a reference to the calling runtime thread, but passing in a `nullptr` will also suffice.

### MMTk-side changes

The MMTk-side of the binding should simply defer the actual implementation to [`mmtk::memory_manager::initialize_collection`](https://docs.mmtk.io/api/mmtk/memory_manager/fn.initialize_collection.html).
See the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk/blob/0ed99cd8cf51bb5ff8184ef64f8236d85e960e87/mmtk/src/api.rs#L245-L248) for an example.

## "Upcalls" Design Pattern

The nature of the bi-directional API means that there are things that MMTk requires or expects from the runtime and vice-versa.
While it is easy for a language runtime to use the API exposed by MMTk (the set of public functions in `mmtk.h`), it is not always easy for the Rust source of MMTk to directly call into the runtime given they may be implemented in a different language.

In order to facilitate this, we utilize a design pattern wherein we define a `struct` of function pointers that is passed on to MMTk during initialization.
These function pointers essentially are the API exposed by the VM to MMTk.
The `struct` is often termed "Upcalls" given MMTk is calling up to the runtime.

Let's take the example of a simple upcall and implement that: getting the size of a given object.

> **Note:** If your runtime is already implemented in Rust, then it should be easy to directly call into your runtime from the MMTk binding, greatly simplifying the bi-directional API.

### Runtime-side changes

We define a new `struct` type with the desired upcall:

```C
[...]

// API from the runtime "Rt" to MMTk
typedef struct {
  size_t (*size_of) (void* object);
} RtUpcalls;

[...]
```

where "`Rt`" is the name of the runtime (for example, OpenJDK would be `OpenjdkUpcalls`, etc.).

We also change the initialization function to take in a pointer to the upcalls:

```C
[...]

/**
 * Initialize MMTk instance
 *
 * @param upcalls the set of Rt upcalls used by MMTk
 */
void mmtk_init(RtUpcalls* upcalls);

[...]
```

Create a new file `mmtk_upcalls.h[pp]` (or whatever the naming scheme of your runtime is) in the runtime-side folder (`mmtk-X/X`) and declare a global instance of the upcalls:

```C
#ifndef MMTK_RT_MMTK_UPCALLS_H
#define MMTK_RT_MMTK_UPCALLS_H

#include "mmtk.h"

// Single global instance of upcalls passed to MMTk
extern RtUpcalls rt_upcalls;

#endif  // MMTK_RT_MMTK_UPCALLS_H
```

This instance is then defined by `mmtk_upcalls.c[pp]`.
We define it like so as all the functions are usually defined as `static` functions (`static` in C/C++ means public in current file, but private to others) to avoid being made public to other users:

```C
#include "mmtk_upcalls.h"  // Use the correct location/name

static size_t size_of(void* object) {
  // Runtime-specific implementation of size_of function
}

RtUpcalls rt_upcalls = {
  size_of,
};
```

The `size_of` function above depends on how your runtime implements getting the size of an object.

Finally, pass the `rt_upcalls` `struct` to where you call the `mmtk_init` function:

```C
[...]

#include "mmtk_upcalls.h"  // Use the correct location/name

// Initialize MMTk
mmtk_init(&rt_upcalls);

[...]
```

### MMTk-side changes

In the MMTk-side of the binding, we need to change the `mmtk_init` API to accept the `RtUpcalls` `struct` as defined above.
We will have to carefully redefine the same `struct` in Rust so that the Rust code can type-check the API correctly.
Unfortunately, this is a brittle approach since you have to carefully maintain the invariant that the Rust and C/C++ definitions of the upcalls `struct` are the same.
An avenue of research that could make it easier and less error-prone would be investigating [`libcxx`](https://cxx.rs/) integration with MMTk.

```Rust
[...]

#[repr(C)]
/// API from the runtime "Rt" to MMTk
pub struct RtUpcalls {
    pub size_of: extern "C" fn(object: ObjectReference) -> usize,
}

/// Global static instance of RtUpcalls
pub static mut UPCALLS: *const RtUpcalls = std::ptr::null_mut();

[...]

pub fn mmtk_init(upcalls: *const RtUpcalls) {
    unsafe { UPCALLS = upcalls };
    // Keep this the same
}

[...]

```

Now, in the `VMObjectModel` trait, we can implement the [`get_current_size`](TODO(kunals)) function:

```Rust
[...]

    fn get_current_size(object: ObjectReference) -> usize {
        use mmtk::util::conversions;
        conversions::raw_align_up(unsafe { ((*UPCALLS).size_of)(object) }, RtName::MIN_ALIGNMENT)
    }

[...]
```

We align the object size to the runtime's minimum alignment in case we want to copy the object while maintaining the alignment requirements.

Astute readers may have noticed that there is an overhead of an indirect call which is not necessarily great for performance.
For performance, we can pull runtime-specific knowledge (such as internal `struct` definitions or state, etc.) into the MMTk-side of the binding to reduce cross-language function calls.
However, this is out of scope for this tutorial as the optimization(s) are highly runtime-dependant.

## Spawning GC Threads

You will notice that now your runtime immediately panics since MMTk is unable to spawn its GC threads. We need to implement the [`VMCollection::spawn_gc_thread`](https://docs.mmtk.io/api/mmtk/vm/trait.Collection.html#tymethod.spawn_gc_thread) API.

Currently there are two kinds of GC threads: the Coordinator thread and GC Worker threads.
There is always only one Coordinator thread and its job is to coordinate GC activities between the different worker threads.
The Coordinator thread does not perform any GC activities itself.
The GC Worker threads actually perform GC activities such as roots scanning, marking objects, etc.
The number of GC Worker threads can be controlled with the [`threads` MMTk option](https://docs.mmtk.io/api/mmtk/util/options/struct.Options.html#structfield.threads) (See the [NoGC guide](./nogc.md#setting-options-for-mmtk) for more information about setting MMTk options).

> **Note:** Since the Coordinator thread always exists, if we set the number of GC threads to 1, the actual number of threads spawned is still 2.

MMTk calls into the runtime to spawn GC threads since there are runtimes that expect all threads to be registered with it.

### Runtime-side changes

Spawning GC threads is highly dependant on your runtime's threading subsystem.
Given MMTk expects the runtime to spawn the threads, we have to implement a new upcall:

In `mmtk.h`:
```C
[...]

// Type of GC worker
enum GcThreadKind {
  MmtkGcController,
  MmtkGcWorker
};

// API from the runtime "Rt" to MMTk
typedef struct {
  size_t (*size_of) (void* object);
  void (*spawn_gc_thread) (void* tls, GcThreadKind kind, void* ctx);
} RtUpcalls;

/**
 * Start the GC Controller thread
 *
 * @param tls the thread that will be used as the GC Controller
 * @param context the context for the GC Controller
 */
void mmtk_start_gc_controller_thread(void* tls, void* context);

/**
 * Start a GC Worker thread
 *
 * @param tls the thread that will be used as the GC Worker
 * @param context the context for the GC Worker
 */
void mmtk_start_gc_worker_thread(void* tls, void* context);

[...]
```

In `mmtk_upcalls.c[pp]`:
```C
[...]

static void spawn_gc_thread(void* tls, GcThreadKind kind, void* ctx) {
  // Runtime-specific implementation of spawning GC worker threads
}

RtUpcalls rt_upcalls = {
  size_of,
  spawn_gc_thread,
};

[...]
```

See the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk/blob/96e868b107b5b13c40c7f4946dff9ac96145c64e/openjdk/mmtkUpcalls.cpp#L95-L121) for an example.

### MMTk-side changes

Define the `GcThreadKind` `enum` in Rust:
```Rust
[...]

/// Type of GC worker
#[repr(C)]
pub enum GcThreadKind {
    /// GC Controller Context thread
    Controller = 0,
    /// Simple GC Worker thread
    Worker     = 1,
}

[...]
```

The MMTk-side changes then should simply call the above upcalls function.
See the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk/blob/96e868b107b5b13c40c7f4946dff9ac96145c64e/mmtk/src/collection.rs#L39-L52) for an example. (Note the OpenJDK binding uses `int`s directly to signify what kind of GC thread is being spawned, but we define the above `enum` which is more sensible).

If your runtime is single-threaded or perhaps it is too difficult to support creating MMTk GC threads, then you could spawn GC threads in the MMTk-side of the binding instead.
For example, MMTk-Ruby does this.

## Suspending (and Resuming) Mutator Threads

The first thing MMTk core does when it finds itself out of memory is block the mutator thread that failed the allocation.
This check only happens in the slow-path (when the runtime goes and gets a new thread-local buffer from MMTk).
You ha

TODO(kunals): VM Companion Thread

### Runtime-side changes
### MMTk-side changes

## Miscellaneous API

TODO(kunals): `mutators`, `get_current_size`, etc.

### Runtime-side changes
### MMTk-side changes

## Scanning Roots

### Thread Roots

### Runtime-specific Roots

### Runtime-side changes
### MMTk-side changes

## Scanning Objects

### Runtime-side changes
### MMTk-side changes

## Miscellaneous API

TODO(kunals): `handle_user_collection_request`, `is_mmtk_object`, `pin_object`, etc.

### Runtime-side changes
### MMTk-side changes
