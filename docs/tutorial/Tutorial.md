# MMTk Tutorial

In this tutorial, you will build multiple garbage collectors using MMTK from scratch. 
**TODO: Finish description.**

## Contents
* [Introduction](#introduction)
* [Preliminaries](#preliminaries)
  * [Set up MMTK and OpenJDK](#set-up-mmtk-and-openjdk)
    * [Basic set up](#basic-set-up)
    * [Set up benchmarks](#set-up-benchmarks)
    * [Working with multiple VM builds](#working-with-multiple-vm-builds)
  * [Create MyGC](#create-mygc)
* [Building a Semispace Collector](#building-a-semispace-collector)
* ?
* [Further Reading](#further-reading)


## Introduction
### What *is* the MMTk?
The Memory Management Toolkit (MMTk) is a framework to design and implement memory managers. It has a core (this repository) written in Rust, and bindings that allow it to work with OpenJDK, V8, and JikesRVM, with more bindings currently in development. The toolkit has a number of pre-built collectors, and is intended to make it relatively simple to expand upon or build new collectors. Many elements common between collectors can be easily implemented.

### What will this tutorial be covering?
This tutorial is intended to get you comfortable with building garbage collectors in the MMTk.

You will first be guided through building a Semispace collector. After that, you will extend this collector in various ways that are not particularly practical, but introduce different concepts implemented in the MMTk. These exersizes will be less guided, but hints and functional solutions will be available in case you get stuck. There will also be questions at various points in the tutorial, intended to encourage you to think about what the code is *doing* and potentially motivate further research.

### Terminology

*allocator*: Handles allocation requests. Allocates objects into memory.

*collector*: Finds and frees memory used by 'dead' objects. 

*dead*: An object that can no longer be accessed by any other object is dead.

*GC work (unit), GC worker*: A worker that performs garbage collection operations using a single thread.

*live*: An object that can still be accessed by other objects is live/alive.

*mutator*: Something that 'mutates', or changes, the objects stored in memory. That is to say, this is a running program.

*plan*: A garbage collection algorithm composed from components.

*policy*: A definition of the semantics and behaviour of a memory region. Memory spaces are instances of policies.

*scheduler*: Schedules GC works so that they can safely be run in parallel.

*work packet*: Contains an instance of a GC worker.

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
2. Clone this repository and the [OpenJDK VM repository](https://github.com/mmtk/openjdk). Place them both in `mmtk-openjdk/repos`.
4. Ensure you can build OpenJDK according to the instructions in the READMEs of [this repository](/../master/README.md) and the [OpenJDK binding repository](https://github.com/mmtk/mmtk-openjdk/blob/master/README.md).



#### Set up benchmarks
A few benchmarks of varying size will be used throughout the tutorial. If you haven't already, set them up now. All of the following commands should be entered in `repos/openjdk`.
1. **HelloWorld** (simplest, will never trigger GC): 
   * Copy the following code into a new Java file titled "HelloWorld.java" in `mmtk-openjdk/repos/openjdk`:
   ```java
   class HelloWorld {
       public static void main(String[] args) {
           System.out.println("Hello World!");
       }
   }
   ```
   * Use the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac HelloWorld.java`.
   * Then, run `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java HelloWorld` to run HelloWorld.
   
2. The Computer Language Benchmarks Game **fannkuchredux** (toy benchmark, allocates a small amount of memory but not enough to trigger a collection): 
   * [Copy this code](https://salsa.debian.org/benchmarksgame-team/benchmarksgame/-/blob/master/bencher/programs/fannkuchredux/fannkuchredux.java) into a new file named "fannkuchredux.java" in `mmtk-openjdk/repos/openjdk`.
   * Use the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac fannkuchredux.java`.
   * Then, run `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java fannkuchredux` to run fannkuchredux.
   
3. **DaCapo** benchmark suite (most complex, will trigger multiple collections): 
   * Fetch using `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.
   * DaCapo contains a variety of benchmarks, but this tutorial will only be using lusearch. Run the lusearch benchmark using the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch` in `repos/openjdk`. 



#### Working with multiple VM builds
**TODO: Fix up this section to reflect build-time GC choice when implemented. See below for draft.**

You will need to build multiple versions of the VM in this tutorial. You should familiarise yourself with how to do this now.
1. To select which garbage collector (GC) plan you would like to use in a given build, you can either use the `MMTK_PLAN` environment variable, or the `--features` flag when building the binding. For example, using `export MMTK_PLAN=semispace` or `--features semispace` will build using the Semispace GC (the default plan). 
2. The build will always generate in `mmtk-openjdk/repos/openjdk/build`. If you would like to keep a build (for instance, to make quick performance comparisons), you can rename either the `build` folder or the folder generated within it (eg `inux-x86_64-normal-server-$DEBUG_LEVEL`). 
   1. Renaming the `build` folder is the safest method for this.
   2. If you rename the internal folder, there is a possibility that the new build will generate incorrectly. If a build appears to generate strangely quickly, it probably generated badly.
   3. A renamed build folder can be tested by changing the file path in commands as appropriate.
   4. If you plan to completely overwrite a build, deleting the folder you are writing over will help prevent errors.
3. Try building using NoGC. Both HelloWorld and the fannkuchredux benchmark should run without issue. If you then run lusearch, it should fail when a collection is triggered. The messages and errors produced should look identical or nearly identical to the log below.
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


**DRAFT FOR RUN-TIME GC CHOICE**
You can select which garbage collection plan to use by adding an argument to a run command (**TODO: wording**). For example, ``TODO: insert actual command here!`` will run the \[TODO] benchmark with the Semispace GC, whereas ``TODO: insert command here!`` will run it with NoGC. 

1. Try using NoGC first. Both HelloWorld and fannkuchredux should run without issue. If you then run lusearch, it should fail when a collection is triggered. The messages and errors produced should look identical or nearly identical to the log below. **TODO: Insert accurate log.**
2. If you haven't already, try using Semispace. lusearch should now pass, as garbage will be collected, and the smaller benchmarks should run the same as they did while using NoGC, as they didn't collect garbage in the first place.
3. When you modify a GC, you will have to rebuild the MMTk core to apply any changes. With many VMs, you will also need to rebuild the VM. 
   1. For OpenJDK, the core will rebuild alongside the VM (**TODO: confirm**).
**A lot of the old section can be reused here.**


### Create MyGC
NoGC is a GC plan that only allocates memory, and does not have a collector. We're going to use it as a base for building a new garbage collector.
1. Each plan is stored in `mmtk-openjdk/repos/mmtk-core/src/plan`. Navigate there and create a copy of the folder `nogc`. Rename it to `mygc`.
3. In *each file* within `mygc`, rename any reference to `nogc` to `mygc`. You will also have to separately rename any reference to `NoGC` to `MyGC`.
   - For example, in Visual Studio Code, you can (making sure case sensitivity is selected in the search function) select one instance of `nogc` and either right click and select "Change all instances" or use the CTRL-F2 shortcut, and then type `mygc`, and repeat for `NoGC`.
4. In order to use MyGC, you will need to make some changes to the following files:
    1. `mmtk-core/src/plan/mod.rs`, under the import statements, add:
    ```rust
    #[cfg(feature = "mygc")]
    pub mod mygc;
    #[cfg(feature = "mygc")]
    pub use self::mygc as selected_plan;
    ```
    2. `mmtk-core/Cargo.toml`, under `#plans`, add: 
    ```rust
    mygc = ["immortalspace", "largeobjectspace"]
    ```
    3. `mmtk-openjdk/mmtk/Cargo.toml`, under `[features]`, add: 
    ```rust 
    mygc = ["mmtk/mygc"] 
    ```
    
Note that all of the above changes almost exactly copy the NoGC entries in each of these files. However, NoGC has some features that are not needed for this tutorial. Remove references to them in the MyGC plan now. 
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

When the tospace is full, the definitions of the spaces are flipped (the 'tospace' becomes a 'fromspace' and vise versa). Then, the collector scans each object in what is now the fromspace. Then, if a live object is found, a copy of it is made in the tospace. That is to say, live objects are copied *from* the fromspace *to* the tospace. After every object is scanned, the fromspace is cleared, and the process begins again. 

### Allocation: Add copyspaces

**TODO: Fix formatting**

The first step of changing the MyGC plan into a Semispace plan is to add the two copyspaces that the collector will allocate memory into. This requires adding two copyspaces, code to properly initialise and prepare the new spaces, and a copy context.
First, in `global.rs`, replace the old immortal space with two copyspaces.
  1. **TODO** change as few imports as possible for this step. Need CommonPlan, AtomicBool, CopySpace. Remove line for allow unused imports. Maybe do these as needed.
  2. Change `pub struct MyGC<VM: VMBinding>` to add new instance variables.
    1. Delete the existing fields in the constructor.
    2. Add `pub hi: AtomicBool,`. This is a thread-safe bool indicating which copyspace is the to-space.
    3. Add `pub copyspace0: CopySpace<VM>,` and `pub copyspace1: CopySpace<VM>,`. These are the two copyspaces.
    4. Add `pub common: CommonPlan<VM>,`. Semispace uses the common plan rather than the base plan. 
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
    3. Create another copyspace, called `copyspace1`, defining it as a fromspace instead of a tospace. (Hint: the definitions for copyspaces are in `mmtk-core/policy/copyspace.rs`.) 
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
       1. Add a helper method to Plan for MyGC called `common` that returns a reference to the common plan.
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
Next, we need to change the mutator, in `mutator.rs`, to allocate to the tospace, and to the two spaces controlled by the common plan. **TODO: import statements**
  1. First, in `lazy_static!`, make the following changes:
     1. Map `Default` to `BumpPointer(0)`.
     2. Map `ReadOnly` to `BumpPointer(1)`.
     3. Map `Los` to `LargeObject(0)`. 
  2. Next, in `create_mygc_mutator`, change which allocator is allocated to what space in `space_mapping`. Note that the space allocation is formatted as a list of tuples.
     1. `BumpPointer(0)` should map to the tospace.
     2. `BumpPointer(1)` should map to `plan.common.get_immortal()`.
     3. `LargeObject(0)` should map to `plan.common.get_los()`.
     4. None of the above should be dereferenced (ie, they should not have the `&` prefix). **TODO: Why?**
There may seem to be 2 extraneous spaces and allocators that have appeared all of a sudden in these past 2 steps. These are parts of the MMTk common plan itself.
 1. The immortal space is used for objects that the virtual machine or a library never expects to move - **TODO: such as?**.
 2. The large object space is needed because MMTk handles particularly large objects differently to normal objects, as the space overhead of copying large objects is very high. Instead, this space is used by a separate GC algorithm in the common plan to avoid having to copy them. 
**TODO: Above was paraphrased from Angus' notes, may need to get more detail or clarification**
**TODO: Does the user need to worry about these going forward?**


With this, you should have the allocation working, but not garbage collection. Try building MyGC now. If you run HelloWorld or Fannkunchredux, they should work. DaCapo's lusearch should fail, as it requires garbage to be collected. 
   
### Collector: Implement garbage collection

**TODO: I don't like how directed this section is, but due to the number of specifics and new concepts here it's a bit difficult.. How much searching should the reader have to do? At the very least, this section should have some better explanations a la the previous section**

  1. Make a new file, called `gc_works`. 
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
  4. Create an implementation block - `impl<VM: VMBinding> CopyContext for MyGCCopyContext<VM>`.
     1. Add a type alias for VMBinding (given to the class as `VM`): `type VM: VM`. 
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
     4. In `init`, set the `tls` variable in the held instance of `mygc` to the one passed to the function. **TODO: Reword.**
     5. In `prepare`, rebind the allocator to the tospace.
     6. Leave `release` with an empty body.
     7. In `alloc`, call the allocator's `alloc` function. Above the function, use an inline attribute (`#[inline(always)]`) to tell the Rust compiler to always inline the function.
     8. In `post_copy` add **TODO: add description**. Also, add an inline (always) attribute. **TODO: Why inline here?**
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
     1. Add a VM type alias (`type VM = VM`).
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
       1. This method should return an ObjectReference, and use the inline (*not* always) attribute.
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
  7. Add two new implementation blocks, `Deref` and `DerefMut` for `MyGCProcessEdges`. **TODO: Finish**
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
     
Next, go back to `mutator.rs`. **TODO: Should this be here? The allocation seems to work without it, but I don't really understand why.**
  1. Create a new function called `mygc_mutator_prepare(_mutator: &mut Mutator <MyGC<VM>>, _tls: OpaquePointer,)`. Its body can stay empty, as there aren't any preparation steps for this GC.
  2. Create a new function called `mygc_mutator_release` that takes the same inputs as the `prepare` function above, and has the following body:
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

Go to `global.rs`.
  1. Find the method `gc_init`. Change this function to initialise the common plan and the two copyspaces, rather than the base plan and mygc_space. The contents of the initializer calls are identical.
  2. Find the method `prepare`. Delete the `unreachable!()` call, and add the following code:
     ```rust
     self.common.prepare(tls, true);
     self.hi
        .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
     let hi = self.hi.load(Ordering::SeqCst); 
     self.copyspace0.prepare(hi);
     self.copyspace1.prepare(!hi);
     ```
     This prepares the common plan, flips the definitions for which space is 'to' and which is 'from', then prepares the copyspaces with the new definition.
  3. Find the method `release`. Delete the `unreachable!()` call, and add the following code:
     ```rust
     self.common.release(tls, true);
     self.fromspace().release();
     ```
  4. Add the following helper method to Plan for MyGC. **TODO: Reword for clarity?**
   ```rust
    fn get_collection_reserve(&self) -> usize {
     self.tospace().reserved_pages()
    }
  ```
  5. Delete `handle_user_collection_request`. This function was an override of a Common plan function, which can run correctly when collection is handled.


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

**TODO: finish this section**

**Idea: Add 2nd older generation exercise**


### Triplespace backup instructions

**TODO: Clean up and check accuracy**

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
