# NoGC

We always start a port with NoGC.
It is the simplest possible plan, simply allocates memory and never collects it.
Although this appears trivial, depending on the complexity of the runtime and how well factored (or not) its internal GC interfaces are, just getting this working may be a major undertaking.
In the case of V8, the refactoring within V8 required to get a simple NoGC plan working was substantial, touching over 100 files.
So it’s a good idea not to underestimate the difficulty of a NoGC port!

In order to implement NoGC, we only need to handle MMTk initialisation (`gc_init`), mutator initialisation (`bind_mutator`), and memory allocation (`alloc`).

You may want to take the following steps.
 
1. Set up the binding repository/directory structure:
    - `/mmtk` - the MMTk side of the binding. To start, this can be an almost direct copy of the [Dummy VM binding](https://github.com/mmtk/mmtk-core/tree/master/vmbindings/dummyvm).
    - `/vm` (rename this to your VM name) - VM-specific code for integrating with MMTk. This should act as a bridge between the generic GC interface offered by the VM and the MMTk side of the binding.
    - You can place your VM repository at any path. For clarity, we assume you will place the VM repo as a sibling of the binding repo. You can also clone `mmtk-core` to a local path, and using
      a local repo of `mmtk-core` will help a lot in your development. So your working directory would look like this (assuming your VM is named as `X`):
      ```
      Your working directory/
      ├─ mmtk-X/
      │  ├─ X/
      │  └─ mmtk/
      ├─ X/
      └─ mmtk-core/ (optional)
      ```
    - You may also find it helpful to take inspiration from the OpenJDK binding, particularly for a more complete example of the relevant `Cargo.toml` files (note: the use of submodules is no longer recommended): https://github.com/mmtk/mmtk-openjdk 
2. Change the VM build process to build and/or link MMTk
    - It may be easier to simply build a static and/or dynamic binary for MMTk and link it to the language directly, manually building new binaries as necessary. 
        1. `cd mmtk-X/mmtk`
        2. `cargo build` to build in debug mode or add `--release` for release mode
        3. Copy the shared or static library from `target/debug` or `target/release` to your desired location
    - Later, you can edit the language build process to build MMTk at the same time automatically.
3. Replace VM allocation with calloc
    - Change all alloc calls in the GC to calloc (https://www.tutorialspoint.com/c_standard_library/c_function_calloc.htm). Note: calloc is used instead of malloc as it zero-initialises memory.
    - The purpose of this step is simply to help you find all allocation calls.
4. Single Threaded MMTk Allocation
    1. Create a `mmtk.h` header file which exposes the functions required to implement NoGC (`gc_init`, `alloc`, `bind_mutator`), and `include` it. You can use the [DummyVM `mmtk.h` header file](https://github.com/mmtk/mmtk-core/blob/master/vmbindings/dummyvm/api/mmtk.h) as an example.
    2. Initialise MMTk by calling `gc_init`, with the size of the heap. In the future, you may wish to make this value configurable via a command line argument or environment variable.
    2. You can set [options for MMTk](https://www.mmtk.io/mmtk-core/mmtk/util/options/struct.Options.html) by using `process` to pass options, or simply by setting environtment variables. For example, to
       use the NoGC plan, you can set the env var `MMTK_PLAN=NoGC`.
    3. Create a MMTk mutator instance using `bind_mutator` and pass the return value of `gc_init`.
    4. Replace all previous `calloc` calls with `alloc` and optionally add a mutex around `alloc` if the VM is multi-threaded. The MMTk handle is the return value of the `bind_mutator` call.
    - In order to perform allocations, you will need to know what object alignment the VM expects. VMs often align allocations at word boundaries (e.g. 4 or 8 bytes) as it allows the CPU to access the data faster at runtime. Additionally, the language may use the unused lowest order bits to store flags (e.g. type information), so it is important that MMTk respects these expectations.
5. Multi Threaded Slow Path MMTk Allocation
    1. Call `bind_mutator` on every thread initialisation and save the handle in the thread local storage.
    2. Remove the mutex around `alloc` and use the stored handle for each thread.
6. Multi Threaded Fast Path MMTk Allocation
    1. Create the MMTk mutator data structure on the VM side to mirror the one in MMTk for each thread. This data structure stores the various allocators that are used for each GC plan. In the case of NoGC, the first bump pointer is the only allocator required.
    2. Copy the contents located at the return value of `bind_mutator` to the created data structure.
    3. Create the ‘fast path’ code in the VM (or replace it if already existing) by incrementing the bump pointer’s cursor stored in the mutator at every allocation. When the cursor hits the limit, trigger MMTk’s `alloc`, which will update the cursor and limit.