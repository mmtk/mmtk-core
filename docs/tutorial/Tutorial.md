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

*plan*: (MMTk-specific) A garbage collection algorithm composed from components.

*policy*: (MMTk-specific) A definition of the semantics and behaviour of a memory region. Memory spaces are instances of policies.

*scheduler*: (MMTk-specific) Schedules GC works so that they can safely be run in parallel.

*work packet*: (MMTk-specific) Contains an instance of a GC worker.

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
A few benchmarks of varying size will be used throughout the tutorial. If you haven't already, set them up now.
1. **HelloWorld** (simplest, will never trigger GC): Copy the following code into a new Java file titled "HelloWorld.java" in `mmtk-openjdk/repos/openjdk`:
   ```java
   class HelloWorld {
       public static void main(String[] args) {
           System.out.println("Hello World!");
       }
   }
   ```
   * Run HelloWorld by using the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac HelloWorld.java` followed by `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java HelloWorld` in `openjdk`.
   
2. The Computer Language Benchmarks Game **fannkuchredux** (toy benchmark, allocates a small amount of memory but not enough to trigger a collection): [Copy this code](https://salsa.debian.org/benchmarksgame-team/benchmarksgame/-/blob/master/bencher/programs/fannkuchredux/fannkuchredux.java) into a new file named "fannkuchredux.java" in `mmtk-openjdk/repos/openjdk`.
   * Run fannkuchredux by using the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac fannkuchredux.java` followed by `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java fannkuchredux` in `openjdk`.
   
3. **DaCapo** benchmark suite (most complex, will trigger multiple collections): Fetch using `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.
   * DaCapo contains a variety of benchmarks, but this tutorial will only be using lusearch. Run the lusearch benchmark using the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch` in `openjdk`. 



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
**TODO: Add section intro**

### Allocation: Add copyspaces
The first step of changing the MyGC plan into a Semispace plan is to add the two copyspaces that the collector will allocate memory into. This requires adding two copyspaces, code to properly initialise and prepare the new spaces, and a copy context.
**I don't like the formatting here. It's cluttered and hard to read.**

1. First, in `global.rs`, replace the old immortal space with two copyspaces.
   1. change as few imports as possible for this step. Need CommonPlan, AtomicBool, CopySpace. Remove line for allow unused imports. Maybe do these as needed.
   2. Change `pub struct MyGC<VM: VMBinding>` to add new instance variables.
      - Delete the two lines in the thing.
      - Add `pub hi: AtomicBool,`. This is a thread-safe bool indicating which copyspace is the to-space.
      - Add `pub copyspace0: CopySpace<VM>,` and `pub copyspace1: CopySpace<VM>,`. These are the two copyspaces.
      - Add `pub common: CommonPlan<VM>,`. Semispace uses the common plan rather than the base plan. 
    3. Change `impl<VM: VMBinding> Plan for MyGC<VM> {`. This section initialises and prepares the objects in MyGC that you just defined.
       - Delete the definition of `mygc_space`. Instead, we will define the two copyspaces here.
       - Define one of the copyspaces by adding the following code: **TODO: Make sure this works. Semispace doesn't use variables, and I'm not confident enough in my rust to say this'll work for sure. But doing it this way makes it easier to write the little excersize below.**
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
         You may have noticed that the CopySpace initialisation requires one more bool compared to ImmortalSpace. 
         
         The definitions for these spaces are stored in `mmtk-core/policy`. By looking in these files, add a comment to the above code noting which bool has what function. Then, copy the above code again, renaming it `copyspace1`, and setting it so that it is a fromspace rather than a tospace.
       - Finally, replace the old MyGC initializer with the following:
       ```rust
       MyGC {
            hi: AtomicBool::new(false),
            copyspace0,
            copyspace1,
            common: CommonPlan::new(vm_map, mmapper, options, heap),
        }
       ```
   4. The plan now has the components it needs for allocation, but not the instructions for how to make use of them.
      - Add a method to Plan for MyGC called `common` that returns a reference to the common plan.
        ```rust
        fn common(&self) -> &CommonPlan<VM> {
          &self.common
        }
        ```
      - Find the method `base` and change it so that it calls the base plan *through* the common plan.
        ```rust
        fn base(&self) -> &BasePlan<VM> {
         &self.common.base
        }
        ```
      - Find the method `gc_init`. Change this function to initialise the common plan and the two copyspaces, rather than the base plan and mygc_space. The contents of the initializer calls are identical.
      - Find the method `prepare`. TODO
      - Find the method `release`. TODO
      - *is this needed here?* Add the following method to Plan for MyGC. **TODO: Find a better way to word this.**
        ```rust
        fn get_collection_reserve(&self) -> usize {
         self.tospace().reserved_pages()
        }
        ```
      - Add a new section of methods for MyGC (outside of the methods for Plan for MyGC).
        ```rust
        impl<VM: VMBinding> MyGC<VM> {
        }
        ```
      - To this, add two methods, `tospace(&self)` and `fromspace(&self)`. They both have return type `&CopySpace<VM>`, and return a reference to the tospace and fromspace respectively. Try writing tospace on your own before looking at the code below, and then write fromspace.
      ```rust
      pub fn tospace(&self) -> &CopySpace<VM> {
        if self.hi.load(Ordering::SeqCst) {
            &self.copyspace1
        } else {
            &self.copyspace0
        }
      }
      ```
   
2. mutator.rs
   * change value maps in lazy_static - going to need different space types for SemiSpace. 
   * create_mygc_mutator: Change space_mapping. tospace gets an immortal space, fromspace gets a large-object space (los). Only from is going to have a collection in it. To and from are swapped each collection, and are of equal size. This means that there’s no chance for tospace to run out of memory, but it isn’t the most efficient system.
3. add mut prep/release functions
4. Test allocation is working
   * How?
   
### Collector: Implement garbage collection
1. Implement work packet. Make new file gc_works. This file implements CopyContext and ProcessEdges. The former provides context for when the gc needs to collect, and ProcessEdges ?
### Adding another copyspace
Less guided exercise: Add “young” copyspace which all new objects go to before moving to the fromspace. No r/w barrier.
Add youngspace (copyspace).
Allocate to youngspace. 
Youngspace gets collected at the same time as the other things.
Live items from youngspace get moved to tospace.
It's an incomplete implementation of a generational GC, in effect.

[**Back to table of contents**](#contents)
***
## Further reading: 
- [MMTk Crate Documentation](https://www.mmtk.io/mmtk-core/mmtk/index.html)
- Original MMTk papers:
  - [*Oil and Water? High Performance Garbage Collection in Java with MMTk*](https://www.mmtk.io/assets/pubs/mmtk-icse-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
  - [*Myths and realities: The performance impact of garbage collection*](https://www.mmtk.io/assets/pubs/mmtk-sigmetrics-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
- [*The Garbage Collection Handbook*](https://learning.oreilly.com/library/view/the-garbage-collection/9781315388007) (Jones, Hosking, Moss, 2016)
- Videos: [MPLR 2020 Keynote](https://www.youtube.com/watch?v=3L6XEVaYAmU), [Deconstructing the Garbage-First Collector](https://www.youtube.com/watch?v=MAk6RdApGLs)
