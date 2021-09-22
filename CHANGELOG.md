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
