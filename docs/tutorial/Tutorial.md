# MMTk Tutorial

In this tutorial, you will build multiple garbage collectors from 
scratch using MMTk. 
You will start with an incredibly simple 'collector' called NoGC, 
and through a series of additions and refinements end up with a 
generational copying garbage collector. 

This tutorial is aimed at GC implementors who would like to implement 
new GC algorithms/plans with MMTk. If you are a language implementor 
interested in *porting* your runtime to MMTk, you should refer to the 
[porting guide](/docs/portingguide/Porting_Guide.md) instead.

This tutorial is a work in progress. Some sections may be rough, and others may 
be missing information (especially about import statements). If something is 
missing or inaccurate, refer to the relevant completed garbage collector if
possible. Please also raise an issue, or create a pull request addressing 
the problem. 


## Contents
* [Introduction](#introduction)
  * [What *is* MMTk?](#what-is-mmtk)
  * [What will this tutorial cover?](#what-will-this-tutorial-cover)
  * [Glossary](#glossary)
    * [Plans and Policies](#plans-and-policies)
* [Preliminaries](#preliminaries)
  * [Set up MMTk and OpenJDK](#set-up-mmtk-and-openjdk)
    * [Basic set up](#basic-set-up)
    * [Test the build](#test-the-build)
    * [Rust Logs](#rust-logs)
    * [Working with multiple VM builds](#working-with-multiple-vm-builds)
  * [Create MyGC](#create-mygc)
* [Building a semispace collector](#building-a-semispace-collector)
  * [What is a semispace collector](#what-is-a-semispace-collector)
  * [Allocation: Add copyspaces](#allocation-add-copyspaces)
  * [Collection: Implement garbage collection](#collection-implement-garbage-collection)
  * [Exercise: Adding another copyspace](#exercise-adding-another-copyspace)
* [Further Reading](#further-reading)


## Introduction
### What *is* MMTk?
The Memory Management Toolkit (MMTk) is a framework for designing and 
implementing memory managers. It has a runtime-neutral core (mmtk-core) 
written in Rust, and bindings that allow it to work with OpenJDK, V8, 
and JikesRVM, with more bindings currently in development. 
MMTk was originally written in Java as part of the Jikes RVM Java runtime.
The current version is similar in its purpose, but was made to be 
very flexible with runtime and able to be ported to many different VMs.

The principal idea of MMTk is that it can be used as a 
toolkit, allowing new GC algorithms to be rapidly developed using 
common components. It also allows different GC algorithms to be 
compared on an apples-to-apples basis, since they share common mechanisms.


### What will this tutorial cover?
This tutorial is intended to get you comfortable constructing new plans in 
MMTk.

You will first be guided through building a semispace collector. After that, 
you will extend this collector to be a generational collector, to further 
familiarise you with different concepts in MMTk. There will also be 
questions and exercises at various points in the tutorial, intended to 
encourage you to think about what the code is doing, increase your general 
understanding of MMTk, and motivate further research.

Where possible, there will be links to finished, functioning code after each 
section so that you can check that your code is correct. Note, however, that 
these will be full collectors. Therefore, there may be some differences between 
these files and your collector due to your position in the tutorial. By the end 
of each major section, your code should be functionally identical to the 
finished code provided.

### Glossary

*allocator*: Code that allocates new objects into memory.

*collector*: Finds and frees memory occupied by 'dead' objects. 

*dead*: An object that is not live.

*GC work (unit), GC packet*: A schedulable unit of collection work. 

*GC worker*: A worker thread that performs garbage collection operations 
(as required by GC work units).

*live*: An object that is reachable, and thus can still be accessed by other 
objects, is live/alive.

*mutator*: Something that 'mutates', or changes, the objects stored in memory. 
This is the term that is traditionally used in the garbage collection literature 
to describe the running program (because it 'mutates' the object graph).

*plan*: A garbage collection algorithm expressed as a configuration of policies.

See also [Plans and policies](#plans-and-policies) below.

*policy*: A specific garbage collection algorithm, such as marksweep, copying, 
immix, etc. Plans are made up of an arrangement of one or more policies. 

See also [Plans and policies](#plans-and-policies) below.

*scheduler*: Dynamically dispatches units of GC work to workers.

*zeroing*, *zero initialization*: Initializing and resetting unused memory 
bits to have a value of 0. Required by most memory-safe programming languages.

See also: [Further Reading](#further-reading)


#### Plans and Policies

In MMTk, collectors are instantiated as plans, which can be thought of as 
configurations of collector policies. In practice, most production 
collectors and almost all collectors in MMTk are comprised of multiple 
algorithms/policies. For example the gencopy plan describes a configuration 
that combines a copying nursery with a semispace mature space. In MMTk we 
think of these as three spaces, each of which happen to use the copyspace 
policy, and which have a relationship which is defined by the gencopy plan. 
Under the hood, gencopy builds upon a common plan which may also contain other 
policies including a space for code, a read-only space, etc.

Thus, someone wishing to construct a new collector based entirely on existing 
policies may be able to do so in MMTk by simply writing a new plan, which is 
what this tutorial covers.

On the other hand, someone wishing to introduce an entirely new garbage 
collection policy (such as Immix, for example), would need to first create 
a policy which specifies that algorithm, before creating a plan which defines 
how the GC algorithm fits together and utilizes that policy.


[**Back to table of contents**](#contents)
***
## Preliminaries
### Set up MMTk and OpenJDK
#### Basic set up
This tutorial can be completed with any binding. However, for the sake of 
simplicity, only the setup for the OpenJDK binding will be described in detail 
here. If you would like to use another binding, you will need to follow the 
README files in their respective repositories 
([JikesRVM](https://github.com/mmtk/mmtk-jikesrvm), 
[V8](https://github.com/mmtk/mmtk-v8))
 to set them up, and find appropriate benchmarks for testing. 
 Also, while it may be useful to fork the relevant repositories to your own 
 account, it is not required for this tutorial.

First, set up OpenJDK, MMTk, and the binding:
1. Clone the OpenJDK binding and mmtk-core repository, and install any relevant
dependancies by following the instructions in the
[OpenJDK binding repository]([OpenJDK binding repository](https://github.com/mmtk/mmtk-openjdk/blob/master/README.md).
2. Ensure you can build OpenJDK according to the instructions in the READMEs of 
[the mmtk-core repository](/../master/README.md) and the 
[OpenJDK binding repository](https://github.com/mmtk/mmtk-openjdk/blob/master/README.md).
   * Use the `slowdebug` option when building the OpenJDK binding. This is the 
   fastest debug variant to build, and allows for easier debugging and better 
   testing. The rest of the tutorial will assume you are using `slowdebug`.
   * You can use the env var `MMTK_PLAN=[PlanName]` to choose a plan to use at run-time.
   The plans that are relavent to this tutorial are `NoGC` and `SemiSpace`.



#### Test the build
A few benchmarks of varying size will be used throughout the tutorial. If you 
haven't already, set them up now. All of the following commands should be 
entered in `repos/openjdk`.
1. **HelloWorld** (simplest, will never trigger GC): 
   1. Copy the following code into a new Java file titled "HelloWorld.java" 
   in `mmtk-openjdk/repos/openjdk`:
   ```java
   class HelloWorld {
       public static void main(String[] args) {
           System.out.println("Hello World!");
       }
   }
   ```
   2. Use the command 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac HelloWorld.java`.
   3. Then, run 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java HelloWorld -XX:+UseThirdPartyHeap` 
   to run HelloWorld.
   4. If your program printed out `Hello World!` as expected, then congratulations, you have MMTk working with OpenJDK!
   
2. The Computer Language Benchmarks Game **fannkuchredux** (micro benchmark, 
allocates a small amount of memory but - depending on heap size and the GC 
plan - may not trigger a collection): 
   1. [Copy this code](https://salsa.debian.org/benchmarksgame-team/benchmarksgame/-/blob/master/bencher/programs/fannkuchredux/fannkuchredux.java) 
   into a new file named "fannkuchredux.java" 
   in `mmtk-openjdk/repos/openjdk`.
   2. Use the command 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac fannkuchredux.java`.
   3. Then, run 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java fannkuchredux -XX:+UseThirdPartyHeap` 
   to run fannkuchredux.
   
3. **DaCapo** benchmark suite (most complex, will likely trigger multiple 
collections): 
   1. Fetch using 
   `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.
   2. DaCapo contains a variety of benchmarks, but this tutorial will only be 
   using lusearch. Run the lusearch benchmark using the command 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch` in `repos/openjdk`. 


#### Rust Logs

By using one of the debug builds, you gain access to the Rust logs - a useful 
tool when testing a plan and observing the general behaviour of MMTk. 
There are two levels of trace that are useful when using MMTk - `trace` 
and `debug`. Generally, `debug` logs information about the slow paths 
(allocation through MMTk, rather than fast path allocation through the binding). 
`trace` includes all the information from `debug`, plus more information about 
both slow and fast paths and garbage collection activities. You can set which 
level to view the logs at by setting the environment variable `RUST_LOG`. For 
more information, see the 
[env_logger crate documentation](https://crates.io/crates/env_logger).
 

#### Working with different GC plans

You will be using multiple GC plans in this tutorial. You should
familiarise yourself with how to do this now.

1. The OpenJDK build will always generate in `mmtk-openjdk/repos/openjdk/build`. From the same
build, you can run different GC plans by using the environment variable `MMTK_PLAN=[PlanName]`.
Generally you won't need multiple VM builds. However, if you
do need to keep a build (for instance, to make quick performance
comparisons), you can do the following: rename either the `build` folder or the folder generated
within it (eg `linux-x86_64-normal-server-$DEBUG_LEVEL`). 
   1. Renaming the `build` folder is the safest method for this.
   2. If you rename the internal folder, there is a possibility that the new 
   build will generate incorrectly. If a build appears to generate strangely 
   quickly, it probably generated badly.
   3. A renamed build folder can be tested by changing the file path in 
   commands as appropriate.
   4. If you plan to completely overwrite a build, deleting the folder you are 
   writing over will help prevent errors.
1. Try running your build with `NoGC`. Both HelloWorld and the fannkuchredux benchmark
should run without issue. If you then run lusearch, it should fail when a 
collection is triggered. It is possible to increase the heap size enough that 
no collections will be triggered, but it is okay to let it fail for now. When 
we build using a proper GC, it will be able to pass. The messages and errors 
produced should look identical or nearly identical to the log below.
    ```
    $ MMTK_PLAN=NoGC ./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch
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
4. Try running your build with `SemiSpace`. lusearch should now
pass, as garbage will be collected, and the smaller benchmarks should run the 
same as they did while using NoGC.
    ```
    MMTK_PLAN=SemiSpace ./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch
    ```



### Create MyGC
NoGC is a GC plan that only allocates memory, and does not have a collector. 
We're going to use it as a base for building a new garbage collector.

Recall that this tutorial will take you through the steps of building a 
collector from basic principles. To do that, you'll create your own plan 
called `MyGC` which you'll gradually refine and improve upon through the 
course of this tutorial. At the beginning MyGC will resemble the very 
simple NoGC plan.

1. Each plan is stored in `mmtk-openjdk/repos/mmtk-core/src/plan`. Navigate 
there and create a copy of the folder `nogc`. Rename it to `mygc`.
3. In *each file* within `mygc`, rename any reference to `nogc` to `mygc`. 
You will also have to separately rename any reference to `NoGC` to `MyGC`.
   * For example, in Visual Studio Code, you can (making sure case sensitivity 
   is selected in the search function) select one instance of `nogc` and either 
   right click and select "Change all instances" or use the CTRL-F2 shortcut, 
   and then type `mygc`, and repeat for `NoGC`.
4. In order to use MyGC, you will need to make some changes to the following 
files. 
    1. `mmtk-core/src/plan/mod.rs`, add:
        ```rust
        pub mod mygc;
        ```
        This adds `mygc` as a module.
    1. `mmtk-core/src/util/options.rs`, add `MyGC` to `PlanSelector`. This allows MMTk to accept `MyGC`
    as a command line option for `plan`, or an environment variable for `MMTK_PLAN`:
        ```rust
        #[derive(Copy, Clone, EnumFromStr, Debug)]
        pub enum PlanSelector {
            NoGC,
            SemiSpace,
            GenCopy,
            MyGC
        }
        ```
    1. `mmtk-core/src/plan/global.rs`, change `create_mutator()` and `create_plan()` to create the `MyGC` mutator and the `MyGC` plan
    based on `PlanSelector`:
        ```rust
        pub fn create_mutator<VM: VMBinding>(
            tls: OpaquePointer,
            mmtk: &'static MMTK<VM>,
        ) -> Box<Mutator<VM>> {
            Box::new(match mmtk.options.plan {
                PlanSelector::NoGC => crate::plan::nogc::mutator::create_nogc_mutator(tls, &*mmtk.plan),
                PlanSelector::SemiSpace => {
                    crate::plan::semispace::mutator::create_ss_mutator(tls, &*mmtk.plan)
                }
                PlanSelector::GenCopy => crate::plan::gencopy::mutator::create_gencopy_mutator(tls, mmtk),
                // Create MyGC mutator based on selector
                PlanSelector::MyGC => crate::plan::mygc::mutator::create_mygc_mutator(tls, &*mmtk.plan),
            })
        }

        pub fn create_plan<VM: VMBinding>(
            plan: PlanSelector,
            vm_map: &'static VMMap,
            mmapper: &'static Mmapper,
            options: Arc<UnsafeOptionsWrapper>,
            scheduler: &'static MMTkScheduler<VM>,
        ) -> Box<dyn Plan<VM = VM>> {
            match plan {
                PlanSelector::NoGC => Box::new(crate::plan::nogc::NoGC::new(
                    vm_map, mmapper, options, scheduler,
                )),
                PlanSelector::SemiSpace => Box::new(crate::plan::semispace::SemiSpace::new(
                    vm_map, mmapper, options, scheduler,
                )),
                PlanSelector::GenCopy => Box::new(crate::plan::gencopy::GenCopy::new(
                    vm_map, mmapper, options, scheduler,
                )),
                // Create MyGC plan based on selector
                PlanSelector::MyGC => Box::new(crate::plan::mygc::MyGC::new(
                    vm_map, mmapper, options, scheduler,
                ))
            }
        }
        ```
    
Note that all of the above changes almost exactly copy the NoGC entries in 
each of these files. However, NoGC has some variants, such as a lock-free 
variant. For simplicity, those are not needed for this tutorial. Remove references to them in
the MyGC plan now. 
1. Within `mygc/global.rs`, find any use of `#[cfg(feature = "mygc_lock_free")]` 
and delete both it *and the line below it*.
2. Then, delete any use of the above line's negation, 
`#[cfg(not(feature = "mygc_lock_free"))]`, this time without changing the 
line below it.

After you rebuild OpenJDK (and `mmtk-core`), you can run MyGC with your new build (`MMTK_PLAN=MyGC`). Try testing it
with the each of the three benchmarks. It should work identically to NoGC.

If you've got to this point, then congratulations! You have created your first working MMTk collector!


At this point, you should familiarise yourself with the MyGC plan if you 
haven't already. Try answering the following questions by looking at the code 
and [Further Reading](#further-reading): 
   * Where is the allocator defined?
   * How many memory spaces are there?
   * What kind of memory space policy is used?
   * What happens if garbage has to be collected?   

[**Back to table of contents**](#contents)



***
## Building a semispace collector
### What is a semispace collector?
In a semispace collector, the heap is divided into two equally-sized spaces, 
called 'semispaces'. One of these is defined as a 'fromspace', and the other 
a 'tospace'. The allocator allocates to the tospace until it is full. 

When the tospace is full, a stop-the-world GC is triggered. The mutator is 
paused, and the definitions of the spaces are flipped (the 'tospace' becomes 
a 'fromspace', and vise versa). Then, the collector scans each object in what 
is now the fromspace. If a live object is found, a copy of it is made in the 
tospace. That is to say, live objects are copied *from* the fromspace *to* 
the tospace. After every object is scanned, the fromspace is cleared. The GC 
finishes, and the mutator is resumed.

### Allocation: Add copyspaces

We will now change your MyGC plan from one that cannot collect garbage
into one that implements the semispace algorithm. The first step of this
is to add the two copyspaces, and allow collectors to allocate memory 
into them. This involves adding two copyspaces, the code to properly initialise 
and prepare the new spaces, and a copy context.


Firstly, change the plan constraints. Some of these constraints are not used 
at the moment, but it's good to set them properly regardless.
1. Look in `plan/plan_constraints.rs`. `PlanConstraints` lists all the possible
options for plan-specific constraints. At the moment, `MYGC_CONSTRAINTS` in `mygc/global.rs` should be using
the default value for `PlanConstraints`. We will make the following changes.
1. Initialize `gc_header_bits` to 2. We reserve 2 bits in the header for GC use.
1. Initialize `moves_objects` to `true`.
1. Initialize `num_specialized_scans` to 1.
[[Finished code (step 1-4]](/docs/tutorial/code/mygc_semispace/global.rs#L45-L51)

Next, in `global.rs`, replace the old immortal (nogc) space with two copyspaces.
1. To the import statement block:
   1. Replace `crate::plan::global::{BasePlan, NoCopy};` with 
   `use crate::plan::global::BasePlan;`. This collector is going to use 
   copying, so there's no point to importing NoCopy anymore.
   2. Add `use crate::plan::global::CommonPlan;`. Semispace uses the common
   plan, which includes an immortal space and a large object space, rather 
   than the base plan. Any garbage collected plan should use `CommonPlan`.
   3. Add `use std::sync::atomic::{AtomicBool, Ordering};`. These are going 
   to be used to store an indicator of which copyspace is the tospace.
   4. Delete `#[allow(unused_imports)]`.
   
   [[Finished code (step 1)]](/docs/tutorial/code/mygc_semispace/global.rs#L1)
   
2. Change `pub struct MyGC<VM: VMBinding>` to add new instance variables.
   1. Delete the existing fields in the constructor.
   2. Add `pub hi: AtomicBool,`. This is a thread-safe bool, indicating which 
   copyspace is the tospace.
   3. Add `pub copyspace0: CopySpace<VM>,` 
   and `pub copyspace1: CopySpace<VM>,`. These are the two copyspaces.
   4. Add `pub common: CommonPlan<VM>,`.
    This holds an instance of the common plan.

   [[Finished code (step 2)]](/docs/tutorial/code/mygc_semispace/global.rs#L36-L41)
  
3. Change `impl<VM: VMBinding> Plan for MyGC<VM>`.
This section initialises and prepares the objects in MyGC that you just defined.
   1. Delete the definition of `mygc_space`. 
   Instead, we will define the two copyspaces here.
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
   3. Create another copyspace, called `copyspace1`, defining it as a fromspace 
   instead of a tospace. (Hint: the definitions for 
   copyspaces are in `src/policy/copyspace.rs`.) 
   4. Finally, replace the old MyGC initializer with the following:
       ```rust
        MyGC {
            hi: AtomicBool::new(false),
            copyspace0,
            copyspace1,
            common: CommonPlan::new(vm_map, mmapper, options, heap, &MYGC_CONSTRAINTS),
        }
       ```

   [[Finished code (step 3-4)]](/docs/tutorial/code/mygc_semispace/global.rs#L147-L168)
   
4. There are a few more functions to add to `Plan for MyGC` next.
     1. Find `gc_init()`. Change it to initialise the common plan and the two 
     copyspaces, rather than the base plan and mygc_space. The contents of the 
     initializer calls are identical.
     
     [[Finished code (step 4 i)]](/docs/tutorial/code/mygc_semispace/global.rs#L71-L80)
     
     2. The trait `Plan` requires a `common()` method that should return a 
     reference to the common plan. Implement this method now.
         ```rust
         fn common(&self) -> &CommonPlan<VM> {
           &self.common
         }
         ```
      3. Find the helper method `base` and change it so that it calls the 
      base plan *through* the common plan.
          ```rust
          fn base(&self) -> &BasePlan<VM> {
            &self.common.base
          }
         ```
      4. Find the method `get_pages_used`. Replace the current body with 
      `self.tospace().reserved_pages() + self.common.get_pages_used()`, to 
      correctly count the pages contained in the tospace and the common plan 
      spaces (which will be explained later).
      5. Also add the following helper function:
         ```rust
         fn get_collection_reserve(&self) -> usize {
             self.tospace().reserved_pages()
         }
         ``` 
      
      [[Finished code (step 4 ii-iv)]](/docs/tutorial/code/mygc_semispace/global.rs#L115-L133)
      
5. Add a new section of methods for MyGC:
    ```rust
    impl<VM: VMBinding> MyGC<VM> {
    }
    ```
   1. To this, add two helper methods, `tospace(&self)` 
   and `fromspace(&self)`. They both have return type `&CopySpace<VM>`, 
   and return a reference to the tospace and fromspace respectively. 
   `tospace()` (see below) returns a reference to the tospace, 
   and `fromspace()` returns a reference to the fromspace.
       ```rust
       pub fn tospace(&self) -> &CopySpace<VM> {
         if self.hi.load(Ordering::SeqCst) {
             &self.copyspace1
         } else {
             &self.copyspace0
         }
       }

        pub fn fromspace(&self) -> &CopySpace<VM> {
            if self.hi.load(Ordering::SeqCst) {
                &self.copyspace0
            } else {
                &self.copyspace1
            }
        }
      ```
   
   [[Finished code (step 5)]](/docs/tutorial/code/mygc_semispace/global.rs#L171-L185)

          
Next, we need to change the mutator, in `mutator.rs`, to allocate to the 
tospace, and to the two spaces controlled by the common plan. 
1. Change the following import statements:
   1. Add `use super::MyGC;`.
   2. Add `use crate::util::alloc::BumpAllocator;`.
   3. Delete `use crate::plan::mygc::MyGC;`.
   
   [[Finished code (step 1)]](/docs/tutorial/code/mygc_semispace/mutator.rs#L1)

2. In `lazy_static!`, make the following changes to `ALLOCATOR_MAPPING`, 
which maps the required allocation semantics to the corresponding allocators. 
For example, for `Default`, we allocate using the first bump pointer allocator 
(`BumpPointer(0)`):
   1. Map `Default` to `BumpPointer(0)`.
   2. Map `ReadOnly` to `BumpPointer(1)`.
   3. Map `Los` to `LargeObject(0)`. 
   
   [[Finished code (step 2)]](/docs/tutorial/code/mygc_semispace/mutator.rs#L47-L51)
   
3. Next, in `create_mygc_mutator`, change which allocator is allocated to what 
space in `space_mapping`. Note that the space allocation is formatted as a list 
of tuples. For example, the first bump pointer allocator (`BumpPointer(0)`) is 
bound with `tospace`. 
   1. `BumpPointer(0)` should map to the tospace.
   2. `BumpPointer(1)` should map to `plan.common.get_immortal()`.
   3. `LargeObject(0)` should map to `plan.common.get_los()`.
   4. None of the above should be dereferenced (ie, they should not have 
   the `&` prefix).
   
   [[Finished code (step 3)]](/docs/tutorial/code/mygc_semispace/mutator.rs#L54-L80)
     
There may seem to be 2 extraneous spaces and allocators that have appeared all 
of a sudden in these past 2 steps. These are parts of the MMTk common plan 
itself.
 1. The immortal space is used for objects that the virtual machine or a 
 library never expects to die.
 2. The large object space is needed because MMTk handles particularly large 
 objects differently to normal objects, as the space overhead of copying 
 large objects is very high. Instead, this space is used by a free list 
 allocator in the common plan to avoid having to copy them. 

With this, you should have the allocation working, but not garbage collection. 
Try building again. If you run HelloWorld or Fannkunchredux, they should
work. DaCapo's lusearch should fail, as it requires garbage to be collected. 
   
   
   
### Collection: Implement garbage collection

#### CopyContext and Scheduler

We need to add a few more things to get garbage collection working. 
Specifically, we need to add a `CopyContext`, which a GC worker uses for 
copying objects, and GC work packets that will be scheduled for a collection.

At the moment, none of the files in the plan are suited for garbage collection 
operations. So, we need to add a new file to hold the `CopyContext` and other 
structures and functions that will give the collector proper functionality.

1. Make a new file under `mygc`, called `gc_work.rs`.
2. In `mod.rs`, import `gc_work` as a module by adding the line `mod gc_work`.
3. In `gc_work.rs`, add the following import statements:
    ```rust
    use super::global::MyGC;
    use crate::policy::space::Space;
    use crate::scheduler::gc_work::*;
    use crate::vm::VMBinding;
    use crate::MMTK;
    use crate::plan::PlanConstraints;
    use crate::scheduler::WorkerLocal;
    ```

4. Add a new structure, `MyGCCopyContext`, with the type parameter 
`VM: VMBinding`. It should have the fields `plan: &'static MyGC<VM>`
and `mygc: BumpAllocator`.
   ```rust
   pub struct MyGCCopyContext<VM: VMBinding> {
       plan:&'static MyGC<VM>,
       mygc: BumpAllocator<VM>,
   }
   ```
   
5. Create an implementation block - 
`impl<VM: VMBinding> CopyContext for MyGCCopyContext<VM>`.
   1. Define the associate type `VM` for `CopyContext` as the VMBinding type 
   given to the class as `VM`: `type VM: VM`. 
   1. Add the following skeleton functions (taken from `plan/global.rs`):
       ```rust
       fn constraints(&self) -> &'static PlanConstraints {
           unimplemented!()
       }
       fn init(&mut self, tls: OpaquePointer) {
           unimplemented!()
       }
       fn prepare(&mut self) {
           unimplemented!()
       }
       fn release(&mut self) {
           unimplemented!()
       }
       fn alloc_copy(`init
           &mut self,
           original: ObjectReference,
           bytes: usize,
           align: usize,
           offset: isize,
           semantics: AllocationSemantics,
       ) -> Address {
           unimplemented!()
       }
       fn post_copy(
           &mut self,
           _obj: ObjectReference,
           _tib: Address,
           _bytes: usize,
           _semantics: AllocationSemantics,
       ) {
           unimplemented!()
       }
       ```
   1. In `init()`, set the `tls` variable in the held instance of `mygc` to
   the one passed to the function.
   1. In `constraints()`, return a reference of `MYGC_CONSTRAINTS`.
   1. We just leave the rest of the functions empty for now and will implement them later.
   1. Add a constructor to `MyGCCopyContext`:
       ```rust
       impl<VM: VMBinding> MyGCCopyContext<VM> {
            pub fn new(mmtk: &'static MMTK<VM>) -> Self {
                Self {
                    plan: &mmtk.plan.downcast_ref::<MyGC<VM>>().unwrap(),
                    mygc: BumpAllocator::new(OpaquePointer::UNINITIALIZED, None, &*mmtk.plan),
                }
            }
        }
       ```
    1. Implement the `WorkerLocal` trait for `MyGCCopyContext`:
        ```rust
        impl<VM: VMBinding> WorkerLocal for MyGCCopyContext<VM> {
            fn init(&mut self, tls: OpaquePointer) {
                CopyContext::init(self, tls);
            }
        }
        ```
   [[Finished code]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L20-L70)
    
6. Add a new public structure, `MyGCProcessEdges`, with the type parameter 
`<VM:VMBinding>`. It will hold an instance of `ProcessEdgesBase` and 
`MyGC`. This is the core part for tracing objects in the `MyGC` plan:
    ```rust
    pub struct MyGCProcessEdges<VM: VMBinding> {
        // Holds a reference to the current plan (Note this will be used in the tracing fast path,
        // and we should not use &dyn Plan here for performance)
        plan: &'static MyGC<VM>,
        base: ProcessEdgesBase<MyGCProcessEdges<VM>>,
    }
    ```
7. Add a new implementations block 
`impl<VM:VMBinding> ProcessEdgesWork for MyGCProcessEdges<VM>`.
   1. Similarly to before, set `ProcessEdgesWork`'s associate type `VM` to 
   the type parameter of `MyGCProcessEdges`, `VM`: `type VM:VM`.
   2. Add a new method, `new`.
       ```rust
        fn new(edges: Vec<Address>, _roots: bool, mmtk: &'static MMTK<VM>) -> Self {
            let base = ProcessEdgesBase::new(edges, mmtk);
            let plan = base.plan().downcast_ref::<MyGC<VM>>().unwrap();
            Self { base, plan }
        }
      ```

8. Now that they've been added, you should import `MyGCCopyContext` and
`MyGCProcessEdges` into `global.rs`, which we will be working in for the
next few steps. [[Finished code]](/docs/tutorial/code/mygc_semispace/global.rs#L1)

9. In `create_worker_local()` in `impl Plan for MyGC`, create an instance of `MyGCCopyContext`:
    ```rust
    fn create_worker_local(
        &self,
        tls: OpaquePointer,
        mmtk: &'static MMTK<Self::VM>,
    ) -> GCWorkerLocalPtr {
        let mut c = MyGCCopyContext::new(mmtk);
        c.init(tls);
        GCWorkerLocalPtr::new(c)
    }
    ```
   
10. `NoCopy` is now no longer needed. Remove it from the import statement block.

11. For the next step, import `crate::scheduler::gc_work::*;`, and modify the
line importing `MMTK` scheduler to read `use crate::scheduler::*;`.
[[Finished code]](/docs/tutorial/code/mygc_semispace/global.rs#L13)

12. Add a new method to `Plan for MyGC`, `schedule_collection()`. This function 
runs when a collection is triggered. It schedules GC work for the plan, i.e.,
it stops all mutators, runs the
scheduler's prepare stage and resumes the mutators. The `StopMutators` work
will invoke code from the bindings to scan threads and other roots, and those scanning work
will further push work for a transitive closure.
    ```rust
    fn schedule_collection(&'static self, scheduler:&MMTkScheduler<VM>) {
        self.base().set_collection_kind();
        self.base().set_gc_status(GcStatus::GcPrepare);
        scheduler.work_buckets[WorkBucketStage::Unconstrained]
            .add(StopMutators::<MyGCProcessEdges<VM>>::new());
        scheduler.work_buckets[WorkBucketStage::Prepare].add(Prepare::<Self, MyGCCopyContext<VM>>::new(self));
        scheduler.work_buckets[WorkBucketStage::Release].add(Release::<Self, MyGCCopyContext<VM>>::new(self));
        scheduler.set_finalizer(Some(EndOfGC));
    }
    ```

#### Prepare for collection

The collector has a number of steps it needs to perform before each collection.
We'll add these now.

1. First, fill in some more of the skeleton functions we added to the 
`CopyContext` (in `gc_work.rs`) earlier:
   1. In `prepare()`, rebind the allocator to the tospace using the function
   `self.mygc.rebind(Some(self.plan.tospace()))`.
   2. In `alloc_copy()`, call the allocator's `alloc` function. Above the function, 
   use an inline attribute (`#[inline(always)]`) to tell the Rust compiler 
   to always inline the function. 
   [[Finished code]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L29-L44)
2. In `global.rs`, find the method `prepare`. Delete the `unreachable!()` 
call, and add the following code:
    ```rust
    self.common.prepare(tls, true);
    self.hi
       .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
    let hi = self.hi.load(Ordering::SeqCst); 
    self.copyspace0.prepare(hi);
    self.copyspace1.prepare(!hi);
    ```
   This function is called at the start of a collection. It prepares the two 
   spaces in the common plan, flips the definitions for which space is 'to' 
   and which is 'from', then prepares the copyspaces with the new definition.
3. Going back to `mutator.rs`, create a new function called 
`mygc_mutator_prepare(_mutator: &mut Mutator <MyGC<VM>>, _tls: OpaquePointer,)`. 
This function will be called at the preparation stage of a collection 
(at the start of a collection) for each mutator. Its body can stay empty, as 
there aren't any preparation steps for the mutator in this GC.
4. In `create_mygc_mutator()`, find the field `prep_func` and change it from
`mygc_mutator_noop()` to `mygc_mutator_prepare()`.


#### Scan objects

Next, we'll add the code to allow the plan to collect garbage - filling out 
functions for work packets.

1. In `gc_work.rs`, add a new method to `ProcessEdgesWork for MyGCProcessEdges`,
`trace_object(&mut self, object: ObjectReference)`.
   1. This method should return an ObjectReference, and use the 
   inline attribute.
   2. Check if the object passed into the function is null 
   (`object.is_null()`). If it is, return the object.
   3. Check if the object is in the tospace 
   (`self.plan().tospace().in_space(object)`). If it is, call `trace_object` 
   through the tospace to check if the object is alive, and return the result:
       ```rust
        #[inline]
        fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
            if object.is_null() {
                return object;
            }
            if self.mygc().tospace().in_space(object) {
                self.mygc().tospace().trace_object::<Self, MyGCCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_MyGC,
                    unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
                )
            } else if self.mygc().fromspace().in_space(object) {
                self.mygc().fromspace().trace_object::<Self, MyGCCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_MyGC,
                    unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
                )
            } else {
                self.mygc().common.trace_object::<Self, MyGCCopyContext<VM>>(self, object)
            }
        }
       ```
   4. If it is not in the tospace, check if the object is in the fromspace 
   and return the result of the fromspace's `trace_object` if it is.
   5. If it is in neither space, forward the call to the common space and let the common space to handle
   object tracing in its spaces (e.g. immortal or large object space):
   `self.mygc().common.trace_object::<Self, MyGCCopyContext<VM>>(self, object)`.

   [[Finished code (step 1)]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L92-L113)
2. Add two new implementation blocks, `Deref` and `DerefMut` for 
`MyGCProcessEdges`. These allow `MyGCProcessEdges` to be dereferenced to 
`ProcessEdgesBase`, and allows easy access to fields in `ProcessEdgesBase`.
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
    
   [[Finished code (step 2)]](/docs/tutorial/code/mygc_semispace/gc_work.rs#L116-L124)
   
3. To `post_copy()`, in the `CopyContext` implementations block, add 
`forwarding_word::clear_forwarding_bits::<VM>(obj);`. Also, add an 
inline attribute.


#### Release and Finalize

Finally, we need to fill out the functions that are, roughly speaking, 
run after each collection.

1. Find the method `release()` in `global.rs`. Replace the 
`unreachable!()` call with the following code:
    ```rust
    self.common.release(tls, true);
    self.fromspace().release();
    ```
    This function is called at the end of a collection. It releases the common
    plan spaces and the fromspace.
2. Go back to `mutator.rs`. In `create_mygc_mutator()`, replace 
`mygc_mutator_noop()` in the `release_func` field with `mygc_mutator_release()`.
3. Leave the `release()` function in the `CopyContext` empty. There are no 
release steps for `CopyContext` in this collector.
4. Create a new function called `mygc_mutator_release()` that takes the same 
inputs as the `prepare()` function above. This function will be called at the 
release stage of a collection (at the end of a collection) for each mutator. 
It rebinds the allocator for the `Default` allocation semantics to the new 
tospace. When the mutator threads resume, any new allocations for `Default` 
will then go to the new tospace. The function has the following body:
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
5. Delete `mygc_mutator_noop()`. It was a placeholder for the prepare and 
release functions that you have now added, so it is now dead code.
6. Delete `handle_user_collection_request()`. This function was an override of 
a Common plan function to ignore user requested collection for NoGC. Now we 
remove it and allow user requested collection.


You should now have MyGC working and able to collect garbage. All three
 benchmarks should be able to pass now. 

If the benchmarks pass - good job! You have built a functional copying
collector!

If you get particularly stuck, instructions for how to complete this exercise
are available [here](#triplespace-backup-instructions).

***

### Exercise: Adding another copyspace

Now that you have a working semispace collector, you should be familiar 
enough with the code to start writing some yourself. The intention of this 
exercise is to reinforce the information from the semispace section, rather 
than to create a useful new collector.

1. Create a copy of your semispace collector, called `triplespace`. 
2. Add a new copyspace to the collector, called the `youngspace`, with the 
following traits:
    * New objects are allocated to the youngspace (rather than the fromspace).
    * During a collection, live objects in the youngspace are moved to the 
    tospace.
    * Garbage is still collected at the same time for all spaces.

When you are finished, try running the benchmarks and seeing how the 
performance of this collector compares to MyGC. Great work!

***

Triplespace is a sort of generational garbage collector. These collectors 
separate out old objects and new objects into separate spaces. Newly 
allocated objects should be scanned far more often than old objects, which 
minimises the time spent repeatedly re-scanning long-lived objects. 

Of course, this means that the Triplespace is incredibly inefficient for a 
generational collector, because the older objects are still being scanned 
every collection. It wouldn't be very useful in a real-life scenario. The 
next thing to do is to make this collector into a more efficient proper 
generational collector.

[**Back to table of contents**](#contents)

***

## Building a copying generational collector

### What is a generational collector?
The *weak generational hypothesis* states that most of the objects allocated
to a heap after one collection will die before the next collection.
Therefore, it is worth separating out 'young' and 'old' objects and only
scanning each as needed, to minimise the number of times old live objects are
scanned. New objects are allocated to a 'nursery', and after one collection
they move to the 'mature' space. In `triplespace`, `youngspace` is a
proto-nursery, and the tospace and fromspace are the mature space.

This collector fixes one of the major problems with semispace - namely, that
any long-lived objects are repeatedly copied back and forth. By separating
these objects into a separate 'mature' space, the number of full heap
collections needed is greatly reduced.


This section is currently incomplete. Instructions for building a
generational copying (gencopy) collector will be added in future.

## Triplespace backup instructions

First, rename all instances of `mygc` to `triplespace`, and add it as a
module by following the instructions in [Create MyGC](#create-mygc).

In `global.rs`:
 1. Add a `youngspace` field to `pub struct TripleSpace`:
       ```rust
       pub struct TripleSpace<VM: VMBinding> {
          pub hi: AtomicBool,
          pub copyspace0: CopySpace<VM>,
          pub copyspace1: CopySpace<VM>,
          pub youngspace: CopySpace<VM>, // Add this!
          pub common: CommonPlan<VM>,
      }
      ```
 2. Define the parameters for the youngspace in `new()` in
 `Plan for TripleSpace`:
      ```rust
      fn new(
         vm_map: &'static VMMap,
         mmapper: &'static Mmapper,
         options: Arc<UnsafeOptionsWrapper>,
         _scheduler: &'static MMTkScheduler<Self::VM>,
     ) -> Self {
         //change - again, completely changed.
         let mut heap = HeapMeta::new(HEAP_START, HEAP_END);

         TripleSpace {
             hi: AtomicBool::new(false),
             copyspace0: CopySpace::new(
                 "copyspace0",
                 false,
                 true,
                 VMRequest::discontiguous(),
                 vm_map,
                 mmapper,
                 &mut heap,
             ),
             copyspace1: CopySpace::new(
                 "copyspace1",
                 true,
                 true,
                 VMRequest::discontiguous(),
                 vm_map,
                 mmapper,
                 &mut heap,
             ),

             // Add this!
             youngspace: CopySpace::new(
                 "youngspace",
                 true,
                 true,
                 VMRequest::discontiguous(),
                 vm_map,
                 mmapper,
                 &mut heap,
             ),
             common: CommonPlan::new(vm_map, mmapper, options, heap, &TRIPLESPACE_CONSTRAINTS),
         }
     }
      ```
 3. Initialise the youngspace in `gc_init()`:
     ```rust
      fn gc_init(
         &mut self,
         heap_size: usize,
         vm_map: &'static VMMap,
         scheduler: &Arc<MMTkScheduler<VM>>,
     ) {
         self.common.gc_init(heap_size, vm_map, scheduler);
         self.copyspace0.init(&vm_map);
         self.copyspace1.init(&vm_map);
         self.youngspace.init(&vm_map); // Add this!
     }
     ```
 4. Prepare the youngspace (as a fromspace) in `prepare()`:
     ```rust
     fn prepare(&self, tls: OpaquePointer) {
        self.common.prepare(tls, true);
        self.hi
            .store(!self.hi.load(Ordering::SeqCst), Ordering::SeqCst);
        let hi = self.hi.load(Ordering::SeqCst);
        self.copyspace0.prepare(hi);
        self.copyspace1.prepare(!hi);
        self.youngspace.prepare(true); // Add this!
    }
     ```
 5. Release the youngspace in `release()`:
     ```rust
     fn release(&self, tls: OpaquePointer) {
        self.common.release(tls, true);
        self.fromspace().release();
        self.youngspace().release(); // Add this!
    }
     ```
 6. Under the reference functions `tospace()` and `fromspace()`, add a similar
 reference function `youngspace()`:
     ```rust
     pub fn youngspace(&self) -> &CopySpace<VM> {
        &self.youngspace
    }
     ```

In `mutator.rs`:
 1. Map a bump pointer to the youngspace (replacing the one mapped to the
  tospace) in `space_mapping` in `create_triplespace_mutator()`:
     ```rust
     space_mapping: box vec![
         (AllocatorSelector::BumpPointer(0), plan.youngspace()), // Change this!
         (
             AllocatorSelector::BumpPointer(1),
             plan.common.get_immortal(),
         ),
         (AllocatorSelector::LargeObject(0), plan.common.get_los()),
     ],
     ```
 2. Rebind the bump pointer to youngspace (rather than the tospace) in
 `triplespace_mutator_release()`:
     ```rust
     pub fn triplespace_mutator_release<VM: VMBinding> (
         mutator: &mut Mutator<VM>,
         _tls: OpaquePointer
     ) {
         let bump_allocator = unsafe {
             mutator
                 .allocators
                 . get_allocator_mut(
                     mutator.config.allocator_mapping[AllocationType::Default]
                 )
             }
             .downcast_mut::<BumpAllocator<VM>>()
             .unwrap();
             bump_allocator.rebind(Some(mutator.plan.youngspace())); // Change this!
     }
     ```

In `gc_work.rs`:
1. Add the youngspace to trace_object, following the same fomat as
 the tospace and fromspace:
    ```rust
        fn trace_object(&mut self, object: ObjectReference) -> ObjectReference {
            if object.is_null() {
                return object;
            }

            // Add this!
            else if self.plan().youngspace().in_space(object) {
                self.plan().youngspace.trace_object::<Self, TripleSpaceCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_TripleSpace,
                    unsafe { self.worker().local::<TripleSpaceCopyContext<VM>>() },
                )
            }

            else if self.plan().tospace().in_space(object) {
                self.plan().tospace().trace_object::<Self, TripleSpaceCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_TripleSpace,
                    unsafe { self.worker().local::<MyGCCopyContext<VM>>() },
                )
            } else if self.plan().fromspace().in_space(object) {
                self.plan().fromspace().trace_object::<Self, MyGCCopyContext<VM>>(
                    self,
                    object,
                    super::global::ALLOC_TripleSpace,
                    unsafe { self.worker().local::<TripleSpaceCopyContext<VM>>() },
                )
            } else {
                self.plan().common.trace_object::<Self, TripleSpaceCopyContext<VM>>(self, object)
            }
        }
    }
    ```


[**Back to table of contents**](#contents)
***
## Further reading:
- [MMTk Crate Documentation](https://www.mmtk.io/mmtk-core/mmtk/index.html)
- Original MMTk papers:
  - [*Oil and Water? High Performance Garbage Collection in Java with MMTk*](https://www.mmtk.io/assets/pubs/mmtk-icse-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
  - [*Myths and realities: The performance impact of garbage collection*](https://www.mmtk.io/assets/pubs/mmtk-sigmetrics-2004.pdf) (Blackburn, Cheng, McKinley, 2004)
- [*The Garbage Collection Handbook*](https://learning.oreilly.com/library/view/the-garbage-collection/9781315388007) (Jones, Hosking, Moss, 2016)
- Videos: [MPLR 2020 Keynote](https://www.youtube.com/watch?v=3L6XEVaYAmU), [Deconstructing the Garbage-First Collector](https://www.youtube.com/watch?v=MAk6RdApGLs)
