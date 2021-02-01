# MMTk Tutorial

In this tutorial, you will build multiple garbage collectors using MMTk from scratch. This tutorial is aimed at GC implementors who would like to implement new GC algorithms/plans using the MMTk.

This tutorial is a work in progress. Some sections may be rough, and others may be missing information (especially about import statements). If something is missing or inaccurate, refer to the relevant completed garbage collector if possible. Please also raise an issue, or create a pull request addressing the problem. 


## Contents
* [Introduction](#introduction)
* [Preliminaries](#preliminaries)
  * [Set up MMTk and OpenJDK](#set-up-mmtk-and-openjdk)
    * [Basic set up](#basic-set-up)
    * [Set up benchmarks](#set-up-benchmarks)
    * [Working with multiple VM builds](#working-with-multiple-vm-builds)
  * [Create MyGC](#create-mygc)
* [Building a Semispace Collector](#building-a-semispace-collector)
* [Further Reading](#further-reading)


## Introduction
### What *is* the MMTk?
The Memory Management Toolkit (MMTk) is a framework to design and implement memory managers. It has a core (mmtk-core) written in Rust, and bindings that allow it to work with OpenJDK, V8, and JikesRVM, with more bindings currently in development. The toolkit has a number of pre-built collectors, and is intended to make it relatively simple to expand upon or build new collectors. Many elements common between collectors can be easily implemented.

### What will this tutorial be covering?
This tutorial is intended to get you comfortable with building garbage collectors in the MMTk.

You will first be guided through building a Semispace collector. After that, you will extend this collector to be a generational collector, to further familiarise you with different concepts in the MMTk. There will also be questions and exersizes at various points in the tutorial, intended to encourage you to think about what the code is doing, increase your general understanding of the MMTk, and motivate further research.

### Terminology

*allocator*: Handles allocation requests. Allocates objects into memory.

*collector*: Finds and frees memory used by 'dead' objects. 

*dead*: An object that can no longer be accessed by any other object is dead.

*GC work (unit), GC packet*: A schedulable unit of collection work. 

*GC worker*: A worker that performs garbage collection operations (as required by GC work units) using a single thread.

*live*: An object that can still be accessed by other objects is live/alive.

*mutator*: Something that 'mutates', or changes, the objects stored in memory. That is to say, this is a running program.

*plan*: A garbage collection algorithm composed of components from the MMTk.

*policy*: A definition of the semantics and behaviour of a memory region. Memory spaces are instances of policies.

*scheduler*: Dynamically dispatches units of GC work to workers.

*zeroing*, *zero initialization*: Initializing and resetting unused memory bits to have a value of 0, generally to improve memory safety.

See also: [Further Reading](#further-reading)

[**Back to table of contents**](#contents)
***
## Preliminaries
### Set up MMTk and OpenJDK
#### Basic set up
This tutorial can be completed with any binding. However, for the sake of simplicity, only the setup for the OpenJDK binding will be described in detail here. If you would like to use another binding, you will need to follow the README files in their respective repositories ([JikesRVM](https://github.com/mmtk/mmtk-jikesrvm), [V8](https://github.com/mmtk/mmtk-v8)) to set them up, and find appropriate benchmarks for testing. Also, while it may be useful to fork the relevant repositories to your own account, it is not required for this tutorial.

First, set up OpenJDK, MMTk, and the binding:
1. Clone the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk).
2. Clone the mmtk-core repository and the [OpenJDK VM repository](https://github.com/mmtk/openjdk). Place them both in `mmtk-openjdk/repos`.
4. Ensure you can build OpenJDK according to the instructions in the READMEs of [the mmtk-core repository](/../master/README.md) and the [OpenJDK binding repository](https://github.com/mmtk/mmtk-openjdk/blob/master/README.md).
   * Use the `slowdebug` option when building the OpenJDK binding. This is the fastest debug variant to build, and allows for easier debugging and better testing. The rest of the tutorial will assume you are using `slowdebug`.



#### Test the build
A few benchmarks of varying size will be used throughout the tutorial. If you haven't already, set them up now. All of the following commands should be entered in `repos/openjdk`.
1. **HelloWorld** (simplest, will never trigger GC): 
   1. Copy the following code into a new Java file titled "HelloWorld.java" in `mmtk-openjdk/repos/openjdk`:
   ```java
   class HelloWorld {
       public static void main(String[] args) {
           System.out.println("Hello World!");
       }
   }
   ```
   2. Use the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac HelloWorld.java`.
   3. Then, run `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java HelloWorld -XX:+UseThirdPartyHeap` to run HelloWorld.
   
2. The Computer Language Benchmarks Game **fannkuchredux** (micro benchmark, allocates a small amount of memory but - depending on heap size and the GC plan - may not trigger a collection): 
   1. [Copy this code](https://salsa.debian.org/benchmarksgame-team/benchmarksgame/-/blob/master/bencher/programs/fannkuchredux/fannkuchredux.java) into a new file named "fannkuchredux.java" in `mmtk-openjdk/repos/openjdk`.
   2. Use the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac fannkuchredux.java`.
   3. Then, run `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java fannkuchredux -XX:+UseThirdPartyHeap` to run fannkuchredux.
   
3. **DaCapo** benchmark suite (most complex, will likely trigger multiple collections): 
   1. Fetch using `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.
   2. DaCapo contains a variety of benchmarks, but this tutorial will only be using lusearch. Run the lusearch benchmark using the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch` in `repos/openjdk`. 

By using one of the debug builds, you gain access to the Rust logs - a useful tool when testing a plan and observing the general behaviour of the MMTk. There are two levels of trace that are useful when using the MMTk - `trace` and `debug`. Generally, `debug` logs information about the slow paths (allocation through MMTk, rather than fast path allocation through the binding). `trace` includes all the information from `debug`, plus more information about both slow and fast paths and garbage collection activities. You can set which level to view the logs at by setting the environment variable `RUST_LOG`. For more information, see the [env_logger crate documentation](https://crates.io/crates/env_logger).
 

#### Working with multiple VM builds

You will need to build multiple versions of the VM in this tutorial. You should familiarise yourself with how to do this now.

1. To select which garbage collector (GC) plan you would like to use in a given build, you will need to export the `MMTK_PLAN` environment variable before building the binding. For example, using `export MMTK_PLAN=semispace` will cause the build to use the Semispace GC (the default plan).
2. The build will always generate in `mmtk-openjdk/repos/openjdk/build`. If you would like to keep a build (for instance, to make quick performance comparisons), you can rename either the `build` folder or the folder generated within it (eg `inux-x86_64-normal-server-$DEBUG_LEVEL`). 
   1. Renaming the `build` folder is the safest method for this.
   2. If you rename the internal folder, there is a possibility that the new build will generate incorrectly. If a build appears to generate strangely quickly, it probably generated badly.
   3. A renamed build folder can be tested by changing the file path in commands as appropriate.
   4. If you plan to completely overwrite a build, deleting the folder you are writing over will help prevent errors.
3. Try building using NoGC. Both HelloWorld and the fannkuchredux benchmark should run without issue. If you then run lusearch, it should fail when a collection is triggered. It is possible to increase the heap size enough that no collections will be triggered, but it is okay to let it fail for now. When we build using a proper GC, it will be able to pass. The messages and errors produced should look identical or nearly identical to the log below.
    ```
    $ ./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch
    Using scaled threading model. 24 processors detected, 24 threads used to drive the workload, in a possible range of [1,64]
    Warning: User attempted a collection request, but it is not supported in NoGC. The request is ignored.
    ===== DaCapo 9.12-MR1 lusearch starting =====
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    thread '<unnamed>' panicked at 'internal error: entered unreachable code: GC triggered in nogc', /opt/rust/toolchains/nightly-2020-07-08-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/src/libstd/macros.rs:16:9
    note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    [2020-12-18T00:27:49Z INFO  mmtk::plan::global]   [POLL] nogc_space: Triggering collection
    fatal runtime error: failed to initiate panic, error 5
    Aborted (core dumped)
    ```
4. If you haven't already, try building using Semispace. lusearch should now pass, as garbage will be collected, and the smaller benchmarks should run the same as they did while using NoGC.



### Create MyGC
NoGC is a GC plan that only allocates memory, and does not have a collector. We're going to use it as a base for building a new garbage collector.
1. Each plan is stored in `mmtk-openjdk/repos/mmtk-core/src/plan`. Navigate there and create a copy of the folder `nogc`. Rename it to `mygc`.
3. In *each file* within `mygc`, rename any reference to `nogc` to `mygc`. You will also have to separately rename any reference to `NoGC` to `MyGC`.
   * For example, in Visual Studio Code, you can (making sure case sensitivity is selected in the search function) select one instance of `nogc` and either right click and select "Change all instances" or use the CTRL-F2 shortcut, and then type `mygc`, and repeat for `NoGC`.
4. In order to use MyGC, you will need to make some changes to the following files. 
    1. `mmtk-core/Cargo.toml`, under `#plans`, add: 
        ```rust
        mygc = ["immortalspace", "largeobjectspace"]
        ```
        This adds a build-time flag for `mygc`, and tells the compiler that `mygc` will use an immortal space and large object space.
    2. `mmtk-core/src/plan/mod.rs`, under the import statements, add:
        ```rust
        #[cfg(feature = "mygc")]
        pub mod mygc;
        #[cfg(feature = "mygc")]
        pub use self::mygc as selected_plan;
        ```
        This adds `mygc` as a module, which can be conditionally compiled using the feature (or environment variable) `mygc`. A GC plan needs to be selected at build time, and only one plan can be selected (as `selected_plan`).
    3. `mmtk-openjdk/mmtk/Cargo.toml`, under `[features]`, add: 
        ```rust 
        mygc = ["mmtk/mygc"] 
        ```
        This adds the build flag to the binding crate, using the `mygc` flag from mmtk-core.
    
Note that all of the above changes almost exactly copy the NoGC entries in each of these files. However, NoGC has some variants, such as a lock-free variant, that are not needed for this tutorial. Remove references to them in the MyGC plan now. 
1. Within `mygc/global.rs`, find any use of `#[cfg(feature = "mygc_lock_free")]` and delete both it *and the line below it*.
2. Then, delete any use of the above line's negation, `#[cfg(not(feature = "mygc_lock_free"))]`, this time without changing the line below it.

After you rebuild OpenJDK (and the MMTk core), you can use MyGC. Try testing it with the each of the three benchmarks. It should work identically to NoGC.

At this point, you should familiarise yourself with the MyGC plan if you haven't already. Try answering the following questions by looking at the code and [Further Reading](#further-reading): 
   * Where is the allocator defined?
   * How many memory spaces are there?
   * What kind of memory space policy is used?
   * What happens if garbage has to be collected?   

[**Back to table of contents**](#contents)



***
## Building a Semispace Collector
### What is a Semispace collector?
In a Semispace collector, the heap is divided into two equally-sized spaces, called 'semispaces'. One of these is defined as a 'fromspace', and the other a 'tospace'. The allocator allocates to the tospace until it is full. 

When the tospace is full, a stop-the-world GC is triggered. The mutator is paused, and the definitions of the spaces are flipped (the 'tospace' becomes a 'fromspace', and vise versa). Then, the collector scans each object in what is now the fromspace. If a live object is found, a copy of it is made in the tospace. That is to say, live objects are copied *from* the fromspace *to* the tospace. After every object is scanned, the fromspace is cleared. The GC finishes, and the mutator is resumed.

### Allocation: Add copyspaces

The first step of changing the MyGC plan into a Semispace plan is to add the two copyspaces and allow collectors to allocate memory into them during collection. This requires adding two copyspaces, code to properly initialise and prepare the new spaces, and a copy context.


Firstly, change the plan constraints. Some of these constraints are not used at the moment, but it's good to set them properly regardless.
1. Look in `plan/plan_constraints.rs`. This file contains all the possible options for constraints. At the moment, `mygc/constraints.rs` contains 2 variables: `GC_HEADER_BITS` and `GC_HEADER_WORDS`. Both are set to 0. Change `GC_HEADER_BITS` to 2.
2. You will need to change two more options: `MOVES_OBJECTS` and `NUM_SPECIALIZED_SCANS`. Copy the lines containing both to `mygc/constraints.rs`.
3. Set `MOVES_OBJECTS` to `true`. 
4. Set `NUM_SPECIALIZED_SCANS` to 1.


Next, in `global.rs`, replace the old immortal space with two copyspaces.
1. To the import statement block:
   1. Replace `crate::plan::global::{BasePlan, NoCopy};` with `use crate::plan::global::BasePlan;`.
   2. Add `use crate::plan::global::CommonPlan;`.
   3. Add `use std::sync::atomic::{AtomicBool, Ordering};`.
   4. Delete `#[allow(unused_imports)]`.
2. Change `pub struct MyGC<VM: VMBinding>` to add new instance variables.
  1. Delete the existing fields in the constructor.
  2. Add `pub hi: AtomicBool,`. This is a thread-safe bool indicating which copyspace is the to-space.
  3. Add `pub copyspace0: CopySpace<VM>,` and `pub copyspace1: CopySpace<VM>,`. These are the two copyspaces.
  4. Add `pub common: CommonPlan<VM>,`. Semispace uses the common plan, which includes an immortal space and a large object space, rather than the base plan. Any garbage collected plan should use `CommonPlan`.
3. Change `impl<VM: VMBinding> Plan for MyGC<VM> {`. This section initialises and prepares the objects in MyGC that you just defined.
  1. Delete the definition of `mygc_space`. Instead, we will define the two copyspaces here.
  2. Define one of the copyspaces by adding the following code: 
      ```rust
       let copyspace0 = CopySpace::new(
            "copyspace0",
            false,
            true,
            VMRequest::discontiguous(),
            vm_map,
            mmapper,
            &mut heap,
        );
      ```
  3. Create another copyspace, called `copyspace1`, defining it as a fromspace instead of a tospace. (Hint: the definitions for copyspaces are in `src/policy/copyspace.rs`.) 
  4. Finally, replace the old MyGC initializer with the following:
      ```rust
       MyGC {
           hi: AtomicBool::new(false),
           copyspace0,
           copyspace1,
           common: CommonPlan::new(vm_map, mmapper, options, heap),
       }
      ```
4. The plan now has the components it needs for allocation, but not the instructions for how to make use of them.
     1. The trait `Plan` requires a `common()` method that returns a reference to the common plan. Implement this method in Plan for MyGC.
         ```rust
         fn common(&self) -> &CommonPlan<VM> {
           &self.common
         }
         ```
      2. Find the helper method `base` and change it so that it calls the base plan *through* the common plan.
          ```rust
          fn base(&self) -> &BasePlan<VM> {
            &self.common.base
          }
         ```
      3. Find the method `get_pages_used`. Replace the current body with `self.tospace().reserved_pages() + self.common.get_pages_used()`, to correctly count the pages contained in the tospace and the common spaces (which will be explained later).

      4. Add a new section of methods for MyGC (outside of the methods for Plan for MyGC).
          ```rust
          impl<VM: VMBinding> MyGC<VM> {
          }
         ```
      5. To this, add two helper methods, `tospace(&self)` and `fromspace(&self)`. They both have return type `&CopySpace<VM>`, and return a reference to the tospace and fromspace respectively. `tospace()` (see below) returns a reference to the tospace, and `fromspace()` returns a reference to the fromspace.
          ```rust
          pub fn tospace(&self) -> &CopySpace<VM> {
            if self.hi.load(Ordering::SeqCst) {
                &self.copyspace1
            } else {
                &self.copyspace0
            }
          }
         ```
      6. Also add the following helper function:
          ```rust
          fn get_collection_reserve(&self) -> usize {
            self.tospace().reserved_pages()
          }
          ``` 
5. Find the method `gc_init`. Change this function to initialise the common plan and the two copyspaces, rather than the base plan and mygc_space. The contents of the initializer calls are identical.
6. Find the method `prepare`. Delete the `unreachable!()` call, and add the following code:
    ```rust
    self.common.prepare(tls, true);
    self.hi
       .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
    let hi = self.hi.load(Ordering::SeqCst); 
    self.copyspace0.prepare(hi);
    self.copyspace1.prepare(!hi);
    ```
   This function is called at the start of a collection. It prepares the two spaces in the common plan, flips the definitions for which space is 'to' and which is 'from', then prepares the copyspaces with the new definition.
7. Find the method `release`. Delete the `unreachable!()` call, and add the following code:
    ```rust
    self.common.release(tls, true);
    self.fromspace().release();
    ```
    This function is called at the end of a collection.

          
Next, we need to change the mutator, in `mutator.rs`, to allocate to the tospace, and to the two spaces controlled by the common plan. 
  1. Change the following import statements:
     1. Add `use super::MyGC;`.
     2. Add `use crate::util::alloc::BumpAllocator;`.
     3. Delete `use crate::plan::nogc::NoGC;`.
     
  1. In `lazy_static!`, make the following changes to `ALLOCATOR_MAPPING`, which maps the required allocation semantics to the corresponding allocators. For example, for `Default`, we allocate using the first bump pointer allocator (`BumpPointer(0)`):
     1. Map `Default` to `BumpPointer(0)`.
     2. Map `ReadOnly` to `BumpPointer(1)`.
     3. Map `Los` to `LargeObject(0)`. 
  2. Next, in `create_mygc_mutator`, change which allocator is allocated to what space in `space_mapping`. Note that the space allocation is formatted as a list of tuples. For example, the first bump pointer allocator (`BumpPointer(0)`) is bound with `tospace`. 
     1. `BumpPointer(0)` should map to the tospace.
     2. `BumpPointer(1)` should map to `plan.common.get_immortal()`.
     3. `LargeObject(0)` should map to `plan.common.get_los()`.
     4. None of the above should be dereferenced (ie, they should not have the `&` prefix).
There may seem to be 2 extraneous spaces and allocators that have appeared all of a sudden in these past 2 steps. These are parts of the MMTk common plan itself.
 1. The immortal space is used for objects that the virtual machine or a library never expects to die.
 2. The large object space is needed because MMTk handles particularly large objects differently to normal objects, as the space overhead of copying large objects is very high. Instead, this space is used by a free list allocator in the common plan to avoid having to copy them. 
 
1. Create a new function called `mygc_mutator_prepare(_mutator: &mut Mutator <MyGC<VM>>, _tls: OpaquePointer,)`. This function will be called at the preparation stage of a collection (at the start of a collection) for each mutator. Its body can stay empty, as there aren't any preparation steps for this GC.
2. Create a new function called `mygc_mutator_release` that takes the same inputs as the `prepare` function above. This function will be called at the release stage of a collection (at the end of a collection) for each mutator. It rebinds the allocator for the `Default` allocation semantics to the new tospace. When the mutator threads resume, any new allocations for `Default` will then go to the new tospace. The function has the following body:
    ```rust
    let bump_allocator = unsafe {
       mutator
           .allocators
           . get_allocator_mut(
               mutator.config.allocator_mapping[AllocationType::Default]
           )
       }
       .downcast_mut::<BumpAllocator<VM>>()
       .unwrap();
       bump_allocator.rebind(Some(mutator.plan.tospace()));
    ```
3. In `create_mygc_mutator`, replace `mygc_mutator_noop` in the `prep_func` and `release_func` fields with `mygc_mutator_prepare` and `mygc_mutator_release` respectively.
4. Delete `mygc_mutator_noop`.



With this, you should have the allocation working, but not garbage collection. Try building MyGC now. If you run HelloWorld or Fannkunchredux, they should work. DaCapo's lusearch should fail, as it requires garbage to be collected. 
   
### Collector: Implement garbage collection

We need to add a few more things to get garbage collection working. Specifically, we need to add a `CopyContext`, which a GC worker uses for copying objects, and GC work packets that will be scheduled for a collection.

1. Make a new file under `mygc`, called `gc_works.rs`. 
2. Add the following import statements:
    ```rust
    use super::global::MyGC;
    use crate::plan::CopyContext;
    use crate::policy::space::Space;
    use crate::scheduler::gc_works::*;
    use crate::util::alloc::{Allocator, BumpAllocator};
    use crate::util::forwarding_word;
    use crate::util::{Address, ObjectReference, OpaquePointer};
    use crate::vm::VMBinding;
    use crate::MMTK;
    use std::marker::PhantomData;
    use std::ops::{Deref, DerefMut};
    ```
3. Add a new structure, `MyGCCopyContext`, with the type parameter `VM: VMBinding`. It should have the fields `plan:&'static MyGC<VM>` and `mygc: BumpAllocator`.
   ```rust
   pub struct MyGCCopyContext<VM: VMBinding> {
       plan:&'static MyGC<VM>,
       mygc: BumpAllocator<VM>,
   }
   ```
4. Create an implementation block - `impl<VM: VMBinding> CopyContext for MyGCCopyContext<VM>`.
   1. Define the associate type `VM` for `CopyContext` as the VMBinding type given to the class as `VM`: `type VM: VM`. 
   2. Add the following skeleton functions (taken from `plan/global.rs`):
       ```rust
       fn new(mmtk: &'static MMTK<Self::VM>) -> Self { };
       fn init(&mut self, tls: OpaquePointer) { };
       fn prepare(&mut self) { };
       fn release(&mut self) { };
       fn alloc_copy(`init
           &mut self,
           original: ObjectReference,
           bytes: usize,
           align: usize,
           offset: isize,
           semantics: AllocationSemantics,
       ) -> Address {
       };
       fn post_copy(
           &mut self,
           _obj: ObjectReference,
           _tib: Address,
           _bytes: usize,
           _semantics: AllocationSemantics,
       ) {
       }
       ```
   3. To `new`, add an initialiser for the class:
       ```rust
       Self {
             plan: &mmtk.plan,
             mygc: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &mmtk.plan),
         }
       ```
   4. In `init`, set the `tls` variable in the held instance of `mygc` to the one passed to the function.
   5. In `prepare`, rebind the allocator to the tospace.
   6. Leave `release` with an empty body.
   7. In `alloc`, call the allocator's `alloc` function. Above the function, use an inline attribute (`#[inline(always)]`) to tell the Rust compiler to always inline the function.
   8. In `post_copy` add the following code. Also, add an inline (always) attribute.
       ```rust
       forwarding_word::clear_forwarding_bits::<VM>(obj);
       ```
5. Add a new public structure, `MyGCProcessEdges`, with the type parameter `<VM:VMBinding>`. It will hold an instance of `ProcessEdgesBase` and `PhantomData`, and implement the Default trait:
    ```rust
    #[derive(Default)]
    pub struct MyGCProcessEdges<VM: VMBinding> {
        base: ProcessEdgesBase<MyGCProcessEdges<VM>>,
        phantom: PhantomData<VM>,
    }
    ```
6. Add a new implementations block `impl<VM:VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM>`.
   1. Similarly to before, set `ProcessEdgesWork`'s associate type `VM` to the type parameter of `MyGCProcessEdges`, `VM`: `type VM:VM`.
   2. Add a new method, `new`.
       ```rust
       fn new(edges: Vec<Address>, _roots: bool) -> Self {
           Self {
               base: ProcessEdgesBase::new(edges),
               ..Default::default()
           }
       }
      ```
   3. Add a new method, `trace_object(&mut self, object: ObjectReference)`.
     1. This method should return an ObjectReference, and use the inline attribute.
     2. Check if the object passed into the function is null (`object.is_null()`). If it is, return the object.
     3. Check if the object is in the tospace (`self.plan().tospace().in_space(object)`). If it is, call `trace_object` through the tospace to check if the object is alive, and return the result:
         ```rust
         self.plan().tospace().trace_object(
               self,
               object,
               super::global::ALLOC_MyGC,
               self.worker().local(),
           )
         ```
     4. If it is not in the tospace, check if the object is in the fromspace and return the result of the fromspace's `trace_object` if it is.
     5. If it is in neither space, it must be in the immortal space, or large object space. Trace the object with `self.plan().common.trace_object(self, object)`.
7. Add two new implementation blocks, `Deref` and `DerefMut` for `MyGCProcessEdges`. These allow `MyGCProcessEdges` to be dereferenced to `ProcessEdgesBase`, and allows easy access to fields in `ProcessEdgesBase`.
    ```rust
    impl<VM: VMBinding> Deref for MyGCProcessEdges<VM> {
        type Target = ProcessEdgesBase<Self>;
        #[inline]
        fn deref(&self) -> &Self::Target {
            &self.base
        }
    }

    impl<VM: VMBinding> DerefMut for MyGCProcessEdges<VM> {
        #[inline]
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.base
        }
    }
    ```
8. A few import statements need to be added to the other files so that they can use the functions in `gc_works`.
   1. `global.rs`: Import `MyGCCopyContext` and `MyGCProcessEdges`.
   2. `mod.rs`: Import `gc_works` as a module (`mod gc_works;`).
   
5. In `global.rs`, delete `handle_user_collection_request`. This function was an override of a Common plan function to ignore user requested collection for NoGC. Now we remove it and allow user requested collection.   


### Adding another copyspace
Now that you have a working Semispace collector, you should be familiar enough with the code to start writing some yourself.
1. Create a copy of your Semispace collector, called `triplespace`. 
2. Add a new copyspace to the collector, called the `youngspace`, with the following traits:
    * New objects are allocated to the youngspace (rather than the fromspace).
    * During a collection, live objects in the youngspace are moved to the tospace.
    * Garbage is still collected at the same time for all spaces.

If you get particularly stuck, instructions for how to complete this exersize are available [here](#triplespace-backup-instructions).

***
Triplespace is a sort of generational garbage collector. These collectors separate out old objects and new objects into separate spaces. Newly allocated objects should be scanned far more often than old objects, which minimises the time spent repeatedly re-scanning long-lived objects. 

Of course, this means that the Triplespace is incredibly inefficient for a generational collector, because the older objects are still being scanned every collection. It wouldn't be very useful in a real-life scenario. The next thing to do is to make this collector into a more efficient proper generational collector.

[**Back to table of contents**](#contents)
***
## Building a copying generational collector

### What is a generational collector?
The *weak generational hypothesis* states that most of the objects allocated to a heap after one collection will die before the next collection. Therefore, it is worth separating out 'young' and 'old' objects and only scanning each as needed, to minimise the number of times old live objects are scanned. New objects are allocated to a 'nursery', and after one collection they move to the 'mature' space. In `triplespace`, `youngspace` is a proto-nursery, and the tospace and fromspace are the mature space.

This collector fixes one of the major problems with Semispace - namely, that any long-lived objects are repeatedly copied back and forth. By separating these objects into a separate 'mature' space, the number of full heap collections needed is greatly reduced.


This section is currently incomplete. 

### Triplespace backup instructions

global.rs:
 - add youngspace to Plan for TripleSpace new()
 - init in gc_init
 - prepare (as fromspace) in prepare()
 - release in release()
 - add reference function fromspace()
 
mutator.rs:
 - add bumppointer to youngspace in space_mapping in create_triplespace_mutator
 - in triplespace_mutator_release: rebind bumpallocator to youngspace
 
gc_works.rs
 - add youngspace to trace_object, following format of to/fromspace


[**Back to table of contents**](#contents)
***
## Further reading: 
- [MMTk Crate Documentation](https://www.mmtk.io/mmtk-core/mmtk/index.html)
- Original MMTk papers:
  - [*Oil and Water? High Performance Garbage Collection in Java with MMTk*](https://www.mmtk.io/assets/pubs/mmtk-icse-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
  - [*Myths and realities: The performance impact of garbage collection*](https://www.mmtk.io/assets/pubs/mmtk-sigmetrics-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
- [*The Garbage Collection Handbook*](https://learning.oreilly.com/library/view/the-garbage-collection/9781315388007) (Jones, Hosking, Moss, 2016)
- Videos: [MPLR 2020 Keynote](https://www.youtube.com/watch?v=3L6XEVaYAmU), [Deconstructing the Garbage-First Collector](https://www.youtube.com/watch?v=MAk6RdApGLs)
