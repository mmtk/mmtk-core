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
