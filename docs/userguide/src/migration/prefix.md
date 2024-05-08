<!--
The canonical location of this document is `mmtk-core/doc/userguide/src/api_migration.md`.
It will be part of the MMTk User Guide, available online at <https://docs.mmtk.io/index.html>.
-->

# API Migration Guide

This document lists changes to the MMTk-VM API that require changes to be made by the VM bindings.
VM binding developers can use this document as a guide to update their code to maintain
compatibility with the latest release of MMTk.

<!--
Developers of mmtk-core:

Add an item when making API-breaking changes, but edit existing item if the same API is changed
again before the upcoming release.  No need to add item if a change only adds new API functions, or
if it is source-compatible with the previous version so that VM binding code does not need to be
changed.

Check the current version in `Cargo.toml` before adding items.
New items should be added to the section for the *upcoming* release.

Use URLs of the pull requests to link to the relevant revisions.  Do not use commit hashes because
they will change after squash-merging.

Maintain a line width of 100 characters so that developers who read this file in an IDE or text
editor can still read comfortably.
-->

## 0.25.0


### `ObjectReference` is no longer nullable

-   mmtk-core PRs:
    -   <https://github.com/mmtk/mmtk-core/pull/1064>
    -   <https://github.com/mmtk/mmtk-core/pull/1130>
-   Binding pull requests for reference
    -   <https://github.com/mmtk/mmtk-openjdk/pull/265>
    -   <https://github.com/mmtk/mmtk-openjdk/pull/273> (for write barriers)

`ObjectReference` can no longer represent a NULL reference.  MMTk now uses `Option<ObjectReference>`
to represent non-existing references, which is more idiomatic in Rust.

API changes:

*   `ObjectReference`
    -   `ObjectReference::NULL` is removed.
    -   `ObjectReference::is_null()` is removed.
    -   `ObjectReference::from_raw_address(addr)` now returns `Option<ObjectReference>`.
        +   It returns `None` if `addr` is not zero.  If you know `addr` cannot be zero, you can use
            the new `ObjectReference::from_raw_address_unchecked(addr)` method instead.

*   `mmtk::memory_manager` (write barriers)
    -   `object_reference_write_pre(mutator, obj, slot, target)`
        +   The `target` parameter is now `Option<ObjectReference>`.
        +   Pass `None` if the slot was holding a NULL reference or any non-reference value.
    -   `object_reference_write_post(mutator, obj, slot, target)`
        +   Same as above.
    -   `object_reference_write(mutator, obj, slot, target)`
        +   It is labelled as `#[deprecated]` and needs to be redesigned.
        +   Before a new version is available, use `object_reference_write_pre` and
            `object_reference_write_post`, instead.

VM bindings need to re-implement the following callbacks:

*   `Edge`
    -   `Edge::load()` now returns `None` if the slot is holding a NULL reference or other
        non-reference values.

*   `ObjectModel`
    -   Some methods used to convert non-zero `Address` to `ObjectReference` using
        `ObjectReference::from_raw_address`.Now they can use
        `ObjectReference::from_raw_address_unchecked` to skip the zero check
    -   `ObjectModel::copy(from, semantics, copy_context)`
        +   It calls `CopyContext::alloc_copy` which always returns non-zero `Address`
    -   `ObjectModel::get_reference_when_copied_to(from, to)`
        +   `to` is never zero because MMTk only calls this after the destination is
            determined.
    -   `ObjectModel::address_to_ref(addr)`
        +   `addr` is never zero because this method is an inverse operation of
            `ref_to_address(objref)` where `objref` is never NULL.

*   `ReferenceGlue`
    -   *Note: If your VM binding is still using `ReferenceGlue` and the reference processor and
        finalization processor in mmtk-core, it is strongly recommended to switch to the
        `Scanning::process_weak_refs` method and implement weak reference and finalization
        processing on the VM side.*
    -   `ReferenceGlue::get_referent` now returns `None` if the referent is cleared.
    -   `ReferenceGlue::clear_referent` now needs to be explicitly implemented.
        +   Note: The `Edge` trait does not have a method for storing NULL to a slot.  The VM
            binding needs to implement its own method to store NULL to a slot.

Miscellaneous changes:

*   Some public API functions still return `Option<ObjectReference>`, but VM bindings can no longer
    wrap them into `extern "C"` functions that return `ObjectReference` because
    `ObjectReference::NULL` is removed.  Use `mmtk::util::api_util::NullableObjectReference` for the
    return value.
    -   `memory_manager::get_finalized_object`
    -   `ObjectReference::get_forwarded_object`


### Instance methods of `ObjectReference` changed

-   mmtk-core PR: <https://github.com/mmtk/mmtk-core/pull/1122>
-   Binding PR for reference: <https://github.com/mmtk/mmtk-openjdk/pull/272>

Some methods of `ObjectReference` are changed.

-   The following methods now require a generic argument `<VM: VMBinding>`:
    -   `ObjectReference::is_reachable`
    -   `ObjectReference::is_live`
    -   `ObjectReference::is_movable`
    -   `ObjectReference::get_forwarded_object`
    -   `ObjectReference::is_in_any_space`
    -   `ObjectReference::is_sane`
-   `ObjectReference::value()` is removed.  Use `ObjectReference::to_raw_address()` instead.


### The GC controller (a.k.a. coordinator) is removed

-   mmtk-core PR: <https://github.com/mmtk/mmtk-core/pull/1067>
-   Binding PR for reference: <https://github.com/mmtk/mmtk-openjdk/pull/268>

The GC controller thread is removed from MMTk core.  Now GC workers can coordinate themselves.

VM bindings need to re-implement the following callbacks:

-   `Collection::spawn_gc_thread`:
    -   It no longer asks the binding to spawn the controller thread.  The VM binding can simply
        remove the code path related to creating the controller thread.
    -   `GCWorker::run` now takes ownership of the `Box<GCWorker>` instance.  The VM binding can
        simply call the method `worker.run()` on the worker instance from
        `GCThreadContext::Worker(worker)`, or pass the worker to the wrapper method
        `mmtk::memory_manager::start_worker()`.

<!--
vim: tw=100
-->
