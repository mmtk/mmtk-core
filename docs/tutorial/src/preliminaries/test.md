# Test the build

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
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap HelloWorld` 
   to run HelloWorld.
   4. If your program printed out `Hello World!` as expected, then 
   congratulations, you have MMTk working with OpenJDK!
   
2. The Computer Language Benchmarks Game **fannkuchredux** (micro benchmark, 
allocates a small amount of memory but - depending on heap size and the GC 
plan - may not trigger a collection): 
   1. [Copy this code](https://salsa.debian.org/benchmarksgame-team/benchmarksgame/-/blob/master/bencher/programs/fannkuchredux/fannkuchredux.java) 
   into a new file named "fannkuchredux.java" 
   in `mmtk-openjdk/repos/openjdk`.
   2. Use the command 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/javac fannkuchredux.java`.
   3. Then, run 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap fannkuchredux` 
   to run fannkuchredux.
   
3. **DaCapo** benchmark suite (most complex, will likely trigger multiple 
collections): 
   1. Fetch using 
   `wget https://sourceforge.net/projects/dacapobench/files/9.12-bach-MR1/dacapo-9.12-MR1-bach.jar/download -O ./dacapo-9.12-MR1-bach.jar`.
   2. DaCapo contains a variety of benchmarks, but this tutorial will only be 
   using lusearch. Run the lusearch benchmark using the command 
   `./build/linux-x86_64-normal-server-$DEBUG_LEVEL/jdk/bin/java -XX:+UseThirdPartyHeap -Xms512M -Xmx512M -jar ./dacapo-9.12-MR1-bach.jar lusearch` in `repos/openjdk`. 


## Rust Logs

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
 

## Working with different GC plans

You will be using multiple GC plans in this tutorial. You should
familiarise yourself with how to do this now.

1. The OpenJDK build will always generate in `mmtk-openjdk/repos/openjdk/build`. 
From the same build, you can run different GC plans by using the environment 
variable `MMTK_PLAN=[PlanName]`. Generally you won't need multiple VM builds. 
However, if you do need to keep a build (for instance, to make quick performance
comparisons), you can do the following: rename either the `build` folder or the 
folder generated within it (eg `linux-x86_64-normal-server-$DEBUG_LEVEL`). 
   1. Renaming the `build` folder is the safest method for this.
   2. If you rename the internal folder, there is a possibility that the new 
   build will generate incorrectly. If a build appears to generate strangely 
   quickly, it probably generated badly.
   3. A renamed build folder can be tested by changing the file path in 
   commands as appropriate.
   4. If you plan to completely overwrite a build, deleting the folder you are 
   writing over will help prevent errors.
1. Try running your build with `NoGC`. Both HelloWorld and the fannkuchredux 
benchmark should run without issue. If you then run lusearch, it should fail 
when a collection is triggered. It is possible to increase the heap size enough 
that no collections will be triggered, but it is okay to let it fail for now. 
When we build using a proper GC, it will be able to pass. The messages and 
errors produced should look identical or nearly identical to the log below.
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