0.16.0 (2022-12-006)
===

Plan
---

* Refactor `MarkSweep` to work with both our native mark sweep policy and the malloc mark sweep policy backed by malloc libraries.

Allocators
---

* Add `FreeListAllocator` which is implemented as a `MiMalloc` allocator.
* Fix a bug in `ImmixAllocator` that alignment is properly taken into consideration when deciding whether to do overflow allocation.

Policies
---

* Add `MarkSweepSpace`:
  * It uses our native MiMalloc implementation in MMTk core.
  * When the feature `malloc_mark_sweep` is enabled, it uses the selected malloc library to back up its allocation.
* Malloc mark sweep now accounts for memory based on page usage, and each malloc library may use a different page size.
* The immix space now uses the newly added `BlockPageResource`.
* Changes to Space Function Table (SFT) to improve our boot time and reduce the memory footprint:
  * Rename the current SFT map to `SFTSparseChunkMap`, and only use it for 32 bits architectures.
  * Add `SFTSpaceMap` and by default use it for 64 bits architectures.
  * Add `SFTDenseChunkMap` and use it when we have no control of the virtual address range for a MMTk space on 64 bits architectures.

API
---

* Add an option `thread_affinity` to set processor affinity for MMTk GC threads.
* Add `AllocationSemantics::NonMoving` to allocate objects that are known to be non-moving at allocation time.
* Add `ReferenceGlue::is_referent_cleared` to allow some bindings to use a special value rather than a normal null reference for a cleared referent.
* Add `pin`, `unpin`, and `is_pinned` for object pinning. Note that some spaces do not support object pinning, and using these methods may
  cause panic if the space does not support object pinning.
* Refactor `ObjectReference`:
  * MMTk core now pervasively uses `ObjectModel::ref_to_address` to get an address from an object reference for setting per-object side metadata.
  * Add `ObjectModel::address_to_ref` that does the opposite of `ref_to_address`: getting an object reference from an address that is returned by `ref_to_address`.
  * Add `ObjectModel::ref_to_header` for the binding to tell us the base header address from an object reference.
  * Rename `ObjectModel::object_start_ref` to `ObjectModel::ref_to_object_start` (to be consistent with other methods).
  * Remove `ObjectModel::OBJECT_REF_OFFSET_BEYOND_CELL`, as we no longer use the raw address of an object reference.
  * Add `ObjectModel::UNIFIED_OBJECT_REFERENCE_ADDRESS`. If a binding uses the same address for `ObjectReference`, `ref_to_address` and `ref_to_object_start`,
    they should set this to `true`. MMTk can utilize this information for optimization.
  * Add `ObjectModel::OBJECT_REF_OFFSET_LOWER_BOUND` to specify the minimam value of the possible offsets between an allocation result and object reference's raw address.
* `destroy_mutator()` no longer requires a boxed mutator as its argument. Instead, a mutable reference to the mutator is required. It is made clear that the binding should
  manage the lifetime of the boxed mutator from a `bind_mutator()` call.
* Remove `VMBinding::LOG_MIN_ALIGNMENT` and `VMBinding::MAX_ALIGNMENT_SHIFT` (so we only keep `VMBinding::MIN_ALIGNMENT` and `VMBinding::MAX_ALIGNMENT`).

Misc
---

* Add a lock-free `BlockPageResource` that can be used for policies that always allocate memory at the granularity of a fixed sized block.
  This page resource facilitates block allocation and reclamation, and uses lock-free operations where possible.
* Fix a race condition in `FreeListPageResource` when multiple threads release pages.
* Fix a bug in `fetch_and/or` in our metadata implementation.
* Fix a bug in side metadata bulk zeroing that may zero unrelated bits if the zeroed region cannot be mapped to whole metadata bytes.
* Remove unused `meta_data_pages_per_region` in page resource implementations.
* Remove the use of `MaybeUninit::uninit().assume_init()` in `FreeListPageResource` which has undefined behaviors
  and causes the illegal instruction error with newer Rust toolchains.
* Remove the trait bound `From<Address>` and `Into<Address>` for `Region`, as we cannot guarantee safe conversion between those two types.
* Extract `Chunk` and `ChunkMap` from the immix policy, and make it available for all the policies.

0.15.0 (2022-09-20)
===

GC Plans
---
* Generational plans now support bounded nursery size and fixed nursery size.
* Immix can be used in a non-moving variant (with no defragmentation) to facilitate early porting stages
  where a non-moving GC is expected by the VM. Enable the feature `immix_no_defrag` to use the variant.
  Note that this variant performs poorly, compared to normal immix.

API
---
* Add `mod build_info` for bindings to get information about the current build.
* Add `trait Edge`. A binding can implement its own edge type if they need more sophisiticated edges than a simple address slot,
  e.g. to support compressed pointers, base pointers with offsets, or tagged pointers.
* Add APIs for implementing write barriers. MMTk provides subusming barriers `object_reference_write()` and pre/post write barriers `object_reference_write_pre/post()`.
* Add APIs for implementing barriers for memory copying such as `array_copy` in Java. MMTk provides `memory_region_copy()` (subsuming) and `memory_region_copy_pre/post()`.
* The `ignore_system_g_c` option is renamed to `ignore_system_gc` to be consistent with our naming convention.
* The `max/min_nursery` option is replaced by `nursery`. Bindings can use `nursery=Fixed:<size>` or `Bounded:<size>` to config the nursery size.
* Metadata access methods now requires a type parameter for the metadata value.
* Metadata compare-exchange methods now return a `Result` rather than a boolean, which is more consistent with Rust atomic types.
* Metadata now supports `fetch_and`, `fetch_or` and `fetch_update`.
* Header metadata access methods in `ObjectModel` now have default implementations.

Misc
---
* Remove all stdout printing in MMTk.
* Fix a bug that `CopySpace` should not try zeroing alloc bit if there is no allocation in the space.
* Fix a few issues in documentation.


0.14.0 (2022-08-08)
===

API
---
* `ProcessEdgesWork` is no longer exposed in the `Scanning` trait. Instead, `RootsWorkFactory` is introduced
  for the bindings to create more work packets.
* `Collection::stop_all_mutators()` now provides a callback `mutator_visitor`. The implementation is required
  to call `mutator_visitor` for each mutator once it is stopped. This requirement was implicit prior to this change.
* Now MMTk creation is done in the builder pattern:
  * `MMTKBuilder` is introduced. Command line argument processing API (`process()` and `process_bulk()`) now
    takes `&MMTKBuilder` as an argument instead of `&MMTK`.
  * `gc_init()` is renamed to `mmtk_init()`. `mmtk_init()` now takes `&MMTKBuilder` as an argument,
    and returns an MMTk instance `Box<MMTK>`.
  * `heap_size` (which used to be an argument for `gc_init()`) is now an MMTk option.
  * All the options now can be set through the command line argument processing API.
* Node enqueuing is supported:
  * Add `Scanning::support_edge_enqueuing()`. A binding may return `false` if they cannot do edge scanning for certain objects.
  * For objects that cannot be enqueued as edges, `Scanning::scan_object_and_trace_edges()` will be called.


Scheduler
---
* Fixed a bug that may cause deadlock when GC workers are parked and the coordinator is still executing work.

Misc
---
* `Plan::gc_init()` and `Space::init()` are removed. Initialization is now properly done in the respective constructors.


0.13.0 (2022-06-27)
===

Allocators
---
* Fixed a bug that in GC stress testing, the allocator slowpath may double count the allocated bytes.
* Fixed a bug that in GC stress testing, the Immix allocator may miss the updates to the allocated bytes in some cases.

Scheduler
---
* Added work stealing mechanisms to the scheduler: a GC worker may steal work packets from other workers.
* Fixed a bug that work buckets may be incorrectly opened when there is still work left in workers' local bucket.

API
---
* Added an associate type `Finalizable` to `ReferenceGlue`, with which, a binding can define their own finalizer type.
* Added a set of malloc APIs that allows a binding to do malloc using MMTk.
* Added `vm_trace_object()` to `ActivePlan`. When tracing an object that is not in any of MMTk spaces, MMTk will call this method
  and allow bindings to handle the object.

Misc
---
* `trait TransitiveClosure` is split into two different traits: `EdgeVisitor` and `ObjectQueue`, and `TransitiveClosure` is now removed.
* Fixed a bug that the work packet statistics were not collected correctly if different work packets used the same display name.
* Fixed a bug that the work packet statistics and the phase statistics use different time units. Now they both use milliseconds.
* Fixed a bug that `acquire_lock` was used to lock a larger scope than what was necessary, which caused bad performance when we have many
  allocation threads (e.g. more than 24 threads).

0.12.0 (2022-05-13)
===

GC Plans
---
* Introduced `trait PlanTraceObject` and procedural macros to derive implementation for it for all the current plans.
* Introduced a work packet type `PlanProcessEdges` that uses `PlanTraceObject`. All the current plans use this type for tracing objects.

Policy
---
* Introduced `trait PolicyTraceObject`. Added an implementation for each policy.

API
---
* Preliminary support for Java-style weak reference is added (set the option `no_reference_types=false` to enable it). Related APIs are slightly changed.
* The type parameter `TransitiveClosure` in `Scanning::scan_object()/scan_objects()` is now replaced with `vm::EdgeVisitor`.
* Minor changes to `Scanning::scan_object()/scan_objects()` so they are more consistent.

Misc
---
* Fixed a bug in object forwarding: an object can leave the being-forwarded state without actually being forwarded, and this
  now won't cause a panic.

0.11.0 (2022-04-01)
===

GC Plans
---
* Introduced a new work packet type `SFTProcessEdges`. Most plans now use `SFTProcessEdges` for tracing objects,
  and no longer need to implement any plan-specific work packet. Mark compact and immix plans still use their own
  tracing work packet.

Policies
---
* Fixed a bug that `ImmixCopyContext` did not set the mark bit after copying an object.
* Fixed a bug that `MarkCompactSpace` used `ObjectReference` and `Address` interchangably. Now `MarkCompactSpace`
  properly deals with `ObjectReference`.

API
---
* `is_mapped_object()` is superseded by `is_in_mmtk_spaces()`. It returns true if the given object reference is in
  MMTk spaces, but it does not guarantee that the object reference actually points to an object.
* `is_mmtk_object()` is added. It can be used to check if an object reference points to an object (useful for conservative stack canning).
  `is_mmtk_object()` is only availble when the `is_mmtk_object` feature is enabled.

Misc
---
* MMTk core now builds with stable Rust toolchains (minimal supported Rust version 1.57.0).
* Fixed a bug that MMTk may not map metadata and SFT for an object reference if the object reference is in a different
  chunk from the allocated address.
* Added `trait Region` and `struct RegionIterator<R>` to allow convenient iteration through memory regions.

0.10.0 (2022-02-14)
===

GC Plans
---
* Removed plan-specific copy contexts. Now each plan needs to provide a configuration for
  `GCWorkerCopyContext` (similar to how they config `Mutator`).
* Fixed a bug that `needs_log_bit` was always set to `true` for generational plans, no matter
  their barrier used the log bit or not.
* Fixed a bug that we may overflow when calculating `get_available_pages()`.

Policies
---
* Refactored copy context. Now a copying policy provides its copy context.
* Mark sweep and mark compact now uses `ObjectIterator` for linear scan.

Scheduler
---
* Introduced `GCController`, a counterpart of `GCWorker`, for the controller thread.
* Refactored `GCWorker`. Now `GCWorker` is seperated into two parts, a thread local part `GCWorker`
  which is owned by GC threads, and a shared part `GCWorkerShared` that is shared between GC threads
  and the scheduler.
* Refactored the creation of the scheduler and the workers to remove some unnecessary `Option<T>` and `RwLock<T>`.

API
---
* Added `process_bulk()` that allows bindings to pass options as a string of key-value pairs.
* `ObjectModel::copy()` now takes `CopySemantics` as a parameter.
* Renamed `Collection::spawn_worker_thread()` to `spawn_gc_thread()`, which is now used to spawn both GC worker and
  GC controller.
* `Collection::out_of_memory()` now takes `AllocationError` as a parameter which hints the binding
  on how to handle the OOM error.
* `Collection::out_of_memory()` now allows a binding to return from the method in the case of a non-critical OOM.
  If a binding returns, `alloc()` will return a zero address.

Misc
---
* Added `ObjectIterator` that provides linear scanning through a region to iterate
  objects using the alloc bit.
* Added a feature `work_packet_stats` to optionally collect work packet statistics. Note that
  MMTk used to always collect work packet statistics.
* Optimized the access to the SFT map.
* Fixed a few issues with documentation.
* The example header file `mmtk.h` now uses the prefix `mmtk_` for all the functions.

0.9.0 (2021-12-16)
===

GC Plans
---
* Added a Lisp2-style mark compact plan.
* Added a GCWorkContext type for each plan which specifies the types used for this plan's GC work packet.
* Changed the allocation semantics mapping for each plan. Now each plan has 1-to-1 mapping between allocation semantics and spaces.

Policies
---
* Fixed a few bugs for Immix space when `DEFRAG` is disabled.

Misc
---
* Added an option `precise_stress` (which defaults to `true`). For precise stress test, MMTk will check for stress GC in
  each allocation (including thread local fastpath allocation). For non-precise stress test, MMTk only checks for stress GC in global allocation.
* Refactored the code about counting scanned stacks to make it easier to read.

0.8.0 (2021-11-01)
===

GC Plans
---
* Added `schedule_common()` to schedule the common work packets for all the plans.
* Added proepr implementation for checking and triggering full heap collection for all the plans.
* Fixed a bug that collection triggers were not properly cleared after a GC.
* Fixed a bug that objects in generational copying's or semispace's tospace were traced.

Policies
---
* Added a parameter `roots: bool` to `ProcessEdgesWork::new()` to indicate whether the packet contains root edges.
* Refactored `ImmixProcessEdges.trace_object()` so it can deal with both defrag GC and fast GC.
* Fixed a bug in Immix that recyclable blocks were not defragment source which could cause OOM without attempting
  to evacuate recyclable blocks.
* Fixed a bug that nursery large objects had their unlogged bits set, and were treated as mature objects.
* Fixed a bug that SFT entries were not set for `MallocSpace`.
* Fixed a bug that SFT entries may not be correctly set if the start address is not chunk aligned.

Allocators
---
* Supported proper stress test for all the allocators.
* Supported proper alignment and offset for `MallocAllocator`.

API
---
* (Breaking change) Renamed `enable_collection()` to `initialize_collection()`.
* Added `enable_collection()` and `disable_collection()`. When MMTk collection is disabled, MMTk allows allocation without
  triggering GCs.
* Added `COORDINATOR_ONLY_STW` to the `Collection` trait. If this is set, the `StopMutators` work can only done by the MMTk
  coordinator thread. Otherwise, any GC thread may be used to stop mutators.

Misc
---
* Added assertions in `extreme_assertions` to check if side metadata access is within their bounds.
* Added `SideMetadataSpec.name` to help debug.
* Added a macro in `util::metadata::side_metadata::spec_defs` to help define side metadata specs without
  explicitly laying out specs and providing offsets for each spec.
* Renamed `SideMetadataSpec.log_min_obj_size` to `SideMetadataSpec.log_bytes_in_region` to avoid ambiguity.
* Fixed some issues and outdated code in the MMTk tutorial.
* Fixed a bug that may cause incorrect perf event values.


0.7.0 (2021-09-22)
===

GC Plans
---
* Refactored to extract common generational code from the existing generational copying plan.
* Added the generational immix plan, a two-generation algorithm that uses immix as its mature generation.

Misc
---
* Upgraded the Rust toolchain we use to nightly-2021-09-17 (rustc 1.57.0-nightly).
* Added a new feature `global_alloc_bit`: mmtk-core will set a bit for each allocated object. This will later be
  used to implement heap iteration and to support tracing internal pointers.
* Refactored the scheduler simplify the implementation by removing the abstract `Scheduler`, `Context` and `WorkerLocal`.
* Renamed the incorrect parameter name `primary` to `full_heap` in a few `prepare()`/`release()` methods.
* Renamed the phases in statistics reports from `mu`(mutator)/`gc` to `other`/`stw`(stop-the-world) so they won't cause
  confusion in concurrenct GC plans.
* Fixed a few misuses of side metadata methods that caused concurrency issues in accessing the unlogged bit.
* Fixed a bug in `MallocSpace` that caused side metadata was not mapped correctly if an object crossed chunk boundary.
* Fixed a bug in `MallocSpace` that it may incorrectly consider a chunk's side metadata is mapped.
* Fixed a bug in side metadata implementation that may cause side metadata not mapped if the side metadata size is less than a page.
* Fixed regression in `LockFreeImmortalSpace`.
* Fixed a few typos in the tutorial.


0.6.0 (2021-08-10)
===

GC Plans
---
* Added the Immix plan, an efficient mark-region garbage collector.
* Added a large code space for BasePlan (included by all the plans).

Allocators
---
* Added the Immix allocator.

Policies
---
* Added the Immix space, and related data structures.
* Added `get_forwarded_object()` and `is_reachable()` to the space function table (SFT).
* Improved the Malloc space (marksweep).
  * It now supports parallel sweeping.
  * It now uses side metadata for page and chunk bits.
  * It now supports bulk check for live objects if mark bits are side metadata.
* Fixed a bug that the side forwarding bits were not cleared for CopySpace.

API
---
* Refactored the metadata specs in the ObjectModel trait. Now each spec
  has a specific type (e.g. VMGlobalLogBitSpec), and provides a simpler constructor
  for binding implementers to create them.
* Added support for VM specific weak reference processing:
  * added an API function `on_closure_end()` to add a callback so the binding will
    be notified when a object transitive closure ends.
  * added `vm_release()` and `process_weak_refs()` to the `Collection` trait.

Misc
---
* Added a few new plan constraints.
  * needs_log_bit: indicates whether a plan will use the global log bit.
  * may_trace_duplicate_edges: indicates whether a plan may allow benign races and trace duplicate edges in a GC.
  * max_non_los_default_alloc_bytes: indicates the maximum object size (in bytes) that can be allocated with the
    default allocator for the plan. For objects that are larger than this, they should be allocated with AllocationSemantics.Los.
* Added utilities to measure process-wide perf events.
* Refactored the offset field for SideMetadataSpec with a union type SideMetadataOffset.
* Fixed a concurrency bug that may cause munprotect fail for the PageProtect plan.
* Fixed a few issues with the tutorial and documentation.

0.5.0 (2021-06-25)
===

GC Plans
---
* Added a new plan PageProtect, a plan to help debugging. It allocates each object
  to a separate page, and protects pages when the pages are released.

API
---
* Major changes to the ObjectModel trait: now a binding must specify each per-object
  metadata used by mmtk-core, whether the metadata resides in header bits provided
  by the VM or side tables provided by mmtk-core. For in-header metadata, a binding
  can further implement how it can be accessed, in case the bits are not always available
  to mmtk-core.

Misc
---
* Refactored metadata to provide unified access to per-object metadata (in-header or side).
* Refactored work packet statistics to allow other types of stats other than execution times.
* Added the feature 'perf_counter' and the option 'perf_events' to collect data from hardware performance counters for work packets.
* 'extreme_assertions' now also checks if values stored in side metadata are correct.
* Fixed a bug that GenCopy may report OOM without doing a full heap GC.
* Fixed a bug that existing mmapping of side metadata memory may get overwritten.
* Fixed a bug that FreeListPageResource may return an incorrect new_chunk flag for the first allocation.

0.4.1 (2021-05-20)
===

Misc
---
* Upgrade the Rust version we use to nightly-2021-05-12 (1.54.0-nightly).

0.4.0 (2021-05-14)
===

API
---
* The type OpaquePointer is now superseded by more specific types (VMThread/VMMutatorThread/VMWorkerThread).
* The methods `ActivePlan::mutator()` and `ActivePlan::is_mutator()` are no longer unsafe due to the change above.
* The method `ActivePlan::worker()` is removed.
* Internal modules are no longer public visible. API functions are provided where necessary.

Misc
---
* Added a feature 'extreme_assertions' that will enable expensive assertions to help debugging.
* Refactored to clean up some unsafe code.
* Improved OOM message for failed mmapping.
* Improved page accounting - the memory used by side metadata is also included and reported.
* Improved worker statistics - the total time per work packet type will be reported.
* Fixed a bug that MMTk may overwrite existing memory mapping.
* Fixed a bug that stress GC was not triggered properly in GenCopy.
* Fixed a bug in FragmentedMmapper that may cause a memory region being mmapped twice.


0.3.2 (2021-04-07)
===

Misc
---
* Changed the dependency of hoard-sys to v0.1.1.
* The dependencies of malloc implementations are optional.

0.3.1 (2021-04-06)
===

Misc
---
* Changed the dependency of hoard-sys to v0.1 from crates.io.


0.3.0 (2021-03-31)
===

GC Plans
---
* Added a marksweep implementation.
* GC plans are now selected at run-time.

Allocators
---
* Added MallocAllocator that can be used as a freelist allocator.
* Added a few implementations of malloc/free that can be chosen as build-time features.

Policies
---
* Added MallocSpace.

API
---
* Added support for finalization.
* HAS_GC_BYTE in the ObjectModel trait is superseded by a feature 'side_gc_byte`.

Misc
---
* Added a general side metadata implementation.
* Added a framework for collecting analysis data.
* Added a framework that allows triggering analysis or sanity GC at byte-granularity.
* Added a tutorial for GC implementors.
* Added a porting guide for MMTk users (language implementors).
* GC workers now cache work locally.
* Fixed concurrency bugs in stack scanning and pointer forwarding.


0.2.0 (2020-12-18)
===

API
---
* Refactored the `ObjectModel` trait and it is clearer now that MMTk expects a GC byte from the VM.
* Removed methods in the API that were marked as deprecated.
* Minor changes to a few methods/traits in API.

Misc
---
* Rewrote the implementation of GC byte and forwarding word due to the API change.
* Calling `gc_init()` will not fail now if the binding has initialized its own logger.
* Fixed a few bugs about incorrect entries in SFT map.
* Fixed wrong allocator config for gencopy.
* Fixed a bug that caused MMTk to panic with OOM in stress tests.
* Fixed a bug that caused the first discontiguous space descriptor being considered as empty.

0.1.0 (2020-11-04)
===

GC Plans
---
Added the following plans:
* NoGC
* SemiSpace
* Generational Copying GC

Allocators
---
Added the following allocators:
* Bump Pointer Allocator
* Large Object Allocator

Policies
---
Added the following space policies:
* Immortal (including variants)
* Large object
* Copy

API
---
* Introduced bi-directional API between VM and MMTk

Misc
---
* Implemented a scheduler, GC work packets and related statistics collecting mechanisms.
* Implemented sanity checking.
