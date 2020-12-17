# MMTK Tutorial
Currently, this file is just dot points to be expanded upon later.
This tutorial is intended to.. **TODO: Finish description.**

## Contents
**TODO: Links to sections go here.**
* [Preliminaries](#preliminaries)
* [Building a Semispace Collector](#building-a-semispace-collector)
* ?
* [Further Reading](#further-reading)

## Preliminaries
### Set up MMTK-core and binding
This tutorial can be completed with any binding. For the sake of simplicity, this tutorial is going to only use the OpenJDK binding. This will only be relevant for this section of the tutorial. To set up one the other bindings, please follow the README files in their respective repos: [JikesRVM](https://github.com/mmtk/mmtk-jikesrvm), [V8](https://github.com/mmtk/mmtk-v8).

It may be useful to fork the below repositories to your own account, but it is not required for this tutorial.
1. Clone the [OpenJDK binding](https://github.com/mmtk/mmtk-openjdk).
2. Clone this repository and the [OpenJDK VM repository](https://github.com/mmtk/openjdk). Place them both in the /repos folder in mmtk-openjdk.
4. Follow the instructions in the README of this repository and the binding repository to make sure they are set up correctly.

A few benchmarks of varying size will be used throughout the tutorial. **TODO: Not sure if this is best placed here.**
1. DeCapo: Fetch using `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.

You will need to build multiple versions of the VM in this tutorial. 
1. To select which garbage collector (GC) plan you would like to use in a given build, you can either use the `MMTK_PLAN` environment variable, or the `--features` flag when building the binding. For example, using `export MMTK_PLAN=semispace` or `--features semispace` will build using the Semispace GC (the default plan). 
2. The build will always be placed in `./build`. If you would like to keep a build, rename the old `./build` folder. By changing the file path in commands, benchmarks can still be run on the  Otherwise, deleting the entire folder before rebuilding will ensure an error-free build. **TODO: Check if this is actually needed - just adding the plan variable to the folder name or not deleting anything in advance would be easier esp for slower machines, but seemed to cause the build to be incomplete when I was doing the pseudo-tutorial.**
3. Try building NoGC. If you then run a DeCapo benchmark, such as `lusearch`, it should fail upon attempting to run a garbage collection.
4. Try building Semispace. The DeCapo benchmark should now pass, as garbage will be collected.

### Create mygc
This tutorial will walk through creating a new garbage collector. For this, you will need to make a copy of NoGC as a base.
1. Each garbage collector plan is stored in `mmtk-openjdk/repos/mmtk-core/src/plan`. Navigate there and create a copy of the folder `nogc`. Rename it to `mygc`.
2. In *each file* within `mygc`, rename any reference to `nogc` to `mygc` (select one occurrence, and then either right click and select "Change all occurrences" or use the shortcut CTRL-F2). You will also have to separately rename any reference to `NoGC` to `MyGC`. 
3. In order to build using `mygc`, you will need to add the following files:
    1. `mmtk-core/src/plan/mod.rs`, under the import statements:
    ```rust
    #[cfg(feature = "mygc")]
    pub mod mygc;
    #[cfg(feature = "mygc")]
    pub use self::mygc as selected_plan;
    ```
    2. `mmtk-core/Cargo.toml`, under `#plans`: 
    ```rust
    mygc = ["immortalspace", "largeobjectspace"]
    ```
    3. `mmtk-openjdk/mmtk/Cargo.toml`, under `[features]`: 
    ```rust 
    mygc = ["mmtk/mygc"] 
    ```
    
Note that all of the above changes almost exactly copy the NoGC entries in each of these files. However, NoGC has some features that are not needed for this tutorial. The below change will not **(TODO: Verify.)** change the functionality of the GC, as the reference to the lock-free feature was removed from the Cargo files above, but will clean up code that is not needed.
1. Within `nogc/global.rs`, find any use of `#[cfg(feature = "nogc_lock_free")]` and delete both it *and the line below it*.
2. Then, find any use of the above line's negation, `#[cfg(not(feature = "nogc_lock_free"))]` and delete *just that line*.

At this point, you should familiarise yourself with the MyGC plan if you haven't already. Try answering the following questions:
   * Where is the allocator?
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
- GC handbook ([O’Reilly access](https://learning.oreilly.com/library/view/the-garbage-collection/9781315388007/?ar))
- Videos: [MPLR 2020 Keynote](https://www.youtube.com/watch?v=3L6XEVaYAmU), [Deconstructing the Garbage-First Collector](https://www.youtube.com/watch?v=MAk6RdApGLs)
- Original MMTK papers
