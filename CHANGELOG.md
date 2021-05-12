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