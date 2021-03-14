# Set up MMTk and OpenJDK

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
dependencies by following the instructions in the
[OpenJDK binding repository](https://github.com/mmtk/mmtk-openjdk/blob/master/README.md).
2. Ensure you can build OpenJDK according to the instructions in the READMEs of 
[the mmtk-core repository](https://github.com/mmtk/mmtk-core/blob/master/README.md) and the 
[OpenJDK binding repository](https://github.com/mmtk/mmtk-openjdk/blob/master/README.md).
   * Use the `slowdebug` option when building the OpenJDK binding. This is the 
   fastest debug variant to build, and allows for easier debugging and better 
   testing. The rest of the tutorial will assume you are using `slowdebug`.
   * You can use the env var `MMTK_PLAN=[PlanName]` to choose a plan to use at run-time.
   The plans that are relevant to this tutorial are `NoGC` and `SemiSpace`.