# MMTK Tutorial

In this tutorial, you will build multiple garbage collectors using MMTK from scratch. 
 
**TODO: Finish description.**

## Contents
* [Preliminaries](#preliminaries)
  * [Set up MMTK and OpenJDK](#set-up-mmtk-and-openjdk)
    * [Basic set up](#basic-set-up)
    * [Set up benchmarks](#set-up-benchmarks)
    * [Working with multiple VM builds](#working-with-multiple-vm-builds)
  * [Create MyGC](#create-mygc)
* [Building a Semispace Collector](#building-a-semispace-collector)
* ?
* [Further Reading](#further-reading)

## Preliminaries
### Set up MMTK and OpenJDK
#### Basic set up
This tutorial can be completed with any binding. However, for the sake of simplicity, only the setup for the OpenJDK binding will be described in detail here. If you would like to use another binding, you will need to follow the README files in their respective repositories ([JikesRVM](https://github.com/mmtk/mmtk-jikesrvm), [V8](https://github.com/mmtk/mmtk-v8)) to set them up, and find appropriate benchmarks for testing. Also, while it may be useful to fork the relevant repositories to your own account, it is not required for this tutorial.

First, set up OpenJDK, MMTK, and the binding:
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
   
3. **DeCapo** benchmark suite (most complex, will trigger multiple collections): Fetch using `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.
   * DeCapo contains a variety of benchmarks, but this tutorial will only be using lusearch. Run the lusearch benchmark using the command `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch` in `openjdk`. 

#### Working with multiple VM builds
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


### Create MyGC
NoGC is a GC plan that only allocates memory, and does not have a collector. We're going to use it as a base for building a new garbage collector.
1. Each plan is stored in `mmtk-openjdk/repos/mmtk-core/src/plan`. Navigate there and create a copy of the folder `nogc`. Rename it to `mygc`.
2. Open up the search menu with CRTL-F. Make sure case-sensitive search is enabled.
3. In *each file* within `mygc`, rename any reference to `nogc` to `mygc` (select one occurrence, and then either right click and select "Change all occurrences" or use the shortcut CTRL-F2). You will also have to separately rename any reference to `NoGC` to `MyGC`. 
4. In order to build using `mygc`, you will need to make some changes to the following files:
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

You can now build MyGC. Try testing it with the each of the three benchmarks. It should work identically to NoGC.

At this point, you should familiarise yourself with the MyGC plan if you haven't already. Try answering the following questions:
**NOTE: These are intended to be really simple questions, mostly aimed at those unfamiliar with garbage collection. They just get the reader to look at the code in the collector and start thinking about how it's working, and hopefully encourage them to do some independant reading if they come across something they don't understand.**
   * Where is the allocator defined?
   * How many memory spaces are there?
   * What kind of memory space policy is used?
   * What happens if garbage has to be collected?
   
[**Back to table of contents**](#contents)
***
## Building a Semispace Collector
### Allocation: Add copyspaces
Add the two copyspaces and change the alloc/mut to work with these spaces
1. global.rs: add imports (CommonPlan, AtomicBool)
   * pub struct MyGC: Remove old. Add copyspaces. Add ‘hi’ to/from indicator. Replace base plan with common plan.
   * impl Plan for MyGC: new: init things. gc_init: init things.
2. mutator.rs
   * change value maps in lazy_static - going to need different space types for SemiSpace. 
   * create_mygc_mutator: Change space_mapping. tospace gets an immortal space, fromspace gets a large-object space (los). Only from is going to have a collection in it. To and from are swapped each collection, and are of equal size. This means that there’s no chance for tospace to run out of memory, but it isn’t the most efficient system.
3. add mut prep/release functions
4. Test allocation is working
   * How?
### Collector: Implement garbage collection
1. Implement work packet. Make new file gc_works. This file implements CopyContext and ProcessEdges. The former provides context for when the gc needs to collect, and ProcessEdges ?
### Adding another copyspace
Less guided exercise: Add “young” copyspace which all new objects go to before moving to the fromspace. 

[**Back to table of contents**](#contents)
***
## Further reading: 
- [MMTK Crate Documentation](https://www.mmtk.io/mmtk-core/mmtk/index.html)
- Original MMTK papers:
  - [*Oil and Water? High Performance Garbage Collection in Java with MMTk*](https://www.mmtk.io/assets/pubs/mmtk-icse-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
  - [*Myths and realities: The performance impact of garbage collection*](https://www.mmtk.io/assets/pubs/mmtk-sigmetrics-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
- [*The Garbage Collection Handbook*](https://learning.oreilly.com/library/view/the-garbage-collection/9781315388007) (Jones, Hosking, Moss, 2016)
- Videos: [MPLR 2020 Keynote](https://www.youtube.com/watch?v=3L6XEVaYAmU), [Deconstructing the Garbage-First Collector](https://www.youtube.com/watch?v=MAk6RdApGLs)
