# MMTK Tutorial

In this tutorial, you will build multiple garbage collectors using MMTK from scratch. 
 
**TODO: Finish description.**

## Contents
* [Preliminaries](#preliminaries)
* [Building a Semispace Collector](#building-a-semispace-collector)
* ?
* [Further Reading](#further-reading)

## Preliminaries
### Set up MMTK and OpenJDK
This tutorial can be completed with any binding. However, for the sake of simplicity, only the setup for the OpenJDK binding will be described in detail here. If you would like to use another binding, you will need to follow the README files in their respective repositories to set them up, and use alternate benchmarks for testing. [JikesRVM](https://github.com/mmtk/mmtk-jikesrvm), [V8](https://github.com/mmtk/mmtk-v8). Also, while it may be useful to fork the relevant repositories to your own account, it is not required for this tutorial.

First, set up OpenJDK, MMTK, and the binding:
1. Clone the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk).
2. Clone this repository and the [OpenJDK VM repository](https://github.com/mmtk/openjdk). Place them both in `mmtk-openjdk/repos`.
4. Follow the instructions in the README of this repository and the binding repository to make sure they are set up correctly. **This feels like a bad way of doing this. Maybe expand on exact sections, since I talk about multiple build setups below.**


You will need to build multiple versions of the VM in this tutorial. 
1. To select which garbage collector (GC) plan you would like to use in a given build, you can either use the `MMTK_PLAN` environment variable, or the `--features` flag when building the binding. For example, using `export MMTK_PLAN=semispace` or `--features semispace` will build using the Semispace GC (the default plan). 
2. The build will always generate in `mmtk-openjdk/repos/openjdk/build`. If you would like to keep a build, you can rename either the `build` folder or the folder generated within it (eg `inux-x86_64-normal-server-$DEBUG_LEVEL`). 
   1. If you rename the internal folder, you *must* add to the start of the folder name, otherwise the build will not generate correctly (e.g. `NOGC_linux-x86_64-normal-server-$DEBUG_LEVEL` will work, but `inux-x86_64-normal-server-$DEBUG_LEVEL-NOGC` will lead to an incomplete build being generated). **I think, at least...**
   2. Renaming the `build` folder will also work, and is less error-prone than renaming the folder within it.
   3. If you plan to completely overwrite a build, deleting the folder you are writing over will help prevent errors.
3. Try building using NoGC. If you then run a benchmark test large enough to trigger a collection, such as DeCapo's `lusearch`, it should fail when the collection is triggered with the error message **TODO: Add error message**. A small test should complete without problems. **TODO: Find or create mini test!**
4. Try building using Semispace. The DeCapo benchmark should now pass, as garbage will be collected, and the **small test** should run the same as it did for NoGC.

A few benchmarks of varying size will be used throughout the tutorial. If you haven't already, set them up now. **TODO: Not sure if this is best placed here.**
1. DeCapo: Fetch using `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.

### Create MyGC
NoGC is a GC plan that only allocates memory, and does not have a collector. We're going to use it as a base for building a new garbage collector.
1. Each plan is stored in `mmtk-openjdk/repos/mmtk-core/src/plan`. Navigate there and create a copy of the folder `nogc`. Rename it to `mygc`.
2. Open up the search menu with CRTL-F. Make sure case-sensitive search is on.
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
1. Within `nogc/global.rs`, find any use of `#[cfg(feature = "mygc_lock_free")]` and delete both it *and the line below it*.
2. Then, delete any use of the above line's negation, `#[cfg(not(feature = "mygc_lock_free"))]`, this time without changing the line below it.

You can now build MyGC. Use the same method as in the binding README, using either `export MMTK_PLAN=mygc` or `--features mygc`.
Once this has compiled, test MyGC with the **small test** and DeCapo benchmark. It should work identically to NoGC. **TODO: Fill out test section**

At this point, you should familiarise yourself with the MyGC plan if you haven't already. Try answering the following questions:
**NOTE: These are intended to be really simple questions, mostly aimed at those unfamiliar with garbage collection. They just get the reader to look at the code in the collector and start thinking about how it's working, and hopefully encourage them to do some independant reading if they come across something they don't understand.**
   * Where is the allocator defined?
   * How many memory spaces are there?
   * What kind of memory space policy is used?
   * What happens if garbage has to be collected?
   * **TODO: Talk about aspects of constructors?**
   
   
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

## Further reading: 
- [MMTK Crate Documentation](https://www.mmtk.io/mmtk-core/mmtk/index.html)
- Original MMTK papers:
  - [*Oil and Water? High Performance Garbage Collection in Java with MMTk*](https://www.mmtk.io/assets/pubs/mmtk-icse-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
  - [*Myths and realities: The performance impact of garbage collection*](https://www.mmtk.io/assets/pubs/mmtk-sigmetrics-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
- [*The Garbage Collection Handbook*](https://learning.oreilly.com/library/view/the-garbage-collection/9781315388007) (Jones, Hosking, Moss, 2016)
- Videos: [MPLR 2020 Keynote](https://www.youtube.com/watch?v=3L6XEVaYAmU), [Deconstructing the Garbage-First Collector](https://www.youtube.com/watch?v=MAk6RdApGLs)
