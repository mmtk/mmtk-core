# API Migration Guide

This document lists changes to the MMTk-VM API that require changes to be made by the VM bindings.
VM binding developers can use this document as a guide to update their code to maintain
compatibility with the latest release of MMTk.


## View control

Collapse details to show only two levels of the list.  This allows you to browse through the
changes quickly and selectively expand the details of your interested items.

<button class="api-migration-details-collapse-all" type="button">Collapse all details</button>
<button class="api-migration-details-expand-all" type="button">Expand all details</button>

<!--

Notes for the mmtk-core developers:

-   Make sure you add to the **upcoming release**.  Check the current version in `Cargo.toml`.
-   You may add new items or edit existing items before a release, whichever makes sense.
-   No need to mention API changes that are source compatible and do not require VM binding code
    to be updated.
-   Use the [template](template.md).
-   100 characters per line.  Those who read this doc in text editors and IDEs will thank you.

-->


## 0.25.0

### `ObjectReference` is no longer nullable

**TL;DR** `ObjectReference` can no longer represent a NULL reference.  Some methods of
`ObjectReferences` and write barrier functions are changed.  VM bindings need to re-implement
methods of the `Edge`, `ObjectModel` and `ReferenceGlue` traits.

API changes:

*   type `ObjectReference`
    -   It can no longer represent NULL reference.
        +   It is now backed by `NonZeroUsize`, and MMTk uses `Option<ObjectReference>` universally
            when an `ObjectReference` may or may not exist.  It is more idiomatic in Rust.
    -   The constant `ObjectReference::NULL` is removed.
    -   `is_null()` is removed.
    -   `from_raw_address(addr)`
        +   The return type is changed to `Option<ObjectReference>`.
        +   It returns `None` if `addr` is not zero.
        +   If you know `addr` cannot be zero, you can use the new method
            `from_raw_address_unchecked(addr)`, instead.
*   module `mmtk::memory_manager`
    -   **Only affects users of write barriers**
    -   `object_reference_write_pre(mutator, obj, slot, target)`
        +   The `target` parameter is now `Option<ObjectReference>`.
        +   Pass `None` if the slot was holding a NULL reference or any non-reference value.
    -   `object_reference_write_post(mutator, obj, slot, target)`
        +   Same as above.
    -   `object_reference_write(mutator, obj, slot, target)`
        +   It is labelled as `#[deprecated]` and needs to be redesigned.  It cannot handle the case
            of storing a non-reference value (such as tagged small integer) into the slot.
        +   Before a replacement is available, use `object_reference_write_pre` and
            `object_reference_write_post`, instead.

VM bindings need to re-implement the following traits:

*   trait `Edge`
    -   `load()`
        +   The return type is changed to `Option<ObjectReference>`.
        +   It returns `None` if the slot is holding a NULL reference or other non-reference
            values.  MMTk will skip those slots.
*   trait `ObjectModel`
    -   `copy(from, semantics, copy_context)`
        +   Previously VM bindings convert the result of `copy_context.alloc_copy()` to
            `ObjectReference` using `ObjectReference::from_raw_address()`.
        +   Because `CopyContext::alloc_copy()` never returns zero, you can use
            `ObjectReference::from_raw_address_unchecked()` to skip the zero check.
    -   `get_reference_when_copied_to(from, to)`
        +   `to` is never zero because MMTk only calls this after the destination is determined.
        +   You may skip the zero check, too.
    -   `address_to_ref(addr)`
        +   `addr` is never zero because this method is an inverse operation of
            `ref_to_address(objref)` where `objref` is never NULL.
        +   You may skip the zero check, too.
*   trait `ReferenceGlue`
    -   *Note: If your VM binding is still using `ReferenceGlue` and the reference processor and
        finalization processor in mmtk-core, it is strongly recommended to switch to the
        `Scanning::process_weak_refs` method and implement weak reference and finalization
        processing on the VM side.*
    -   `get_referent()`
        +   The return type is changed to `Option<ObjectReference>`.
        +   It now returns `None` if the referent is cleared.
    -   `clear_referent()`
        +   It now needs to be explicitly implemented because `ObjectReference::NULL` no longer
            exists.
        +   Note: The `Edge` trait does not have a method for storing NULL to a slot.  The VM
            binding needs to implement its own method to store NULL to a slot.

Miscellaneous changes:

*   Some public API functions still return `Option<ObjectReference>`, but VM bindings can no longer
    wrap them into `extern "C"` functions that return `ObjectReference` because
    `ObjectReference::NULL` is removed.  Use `mmtk::util::api_util::NullableObjectReference` for the
    return value.
    -   `memory_manager::get_finalized_object`
    -   `ObjectReference::get_forwarded_object`

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1064>
-   PR: <https://github.com/mmtk/mmtk-core/pull/1130> (for write barriers)
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/265>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/273> (for write barriers)


### Instance methods of `ObjectReference` changed

**TL;DR** Some methods of `ObjectReference` are changed.

API changes:

*   type `ObjectReference`
    -   The following methods now require a generic argument `<VM: VMBinding>`:
        -   `ObjectReference::is_reachable`
        -   `ObjectReference::is_live`
        -   `ObjectReference::is_movable`
        -   `ObjectReference::get_forwarded_object`
        -   `ObjectReference::is_in_any_space`
        -   `ObjectReference::is_sane`
    -   `ObjectReference::value()` is removed.
        +   Use `ObjectReference::to_raw_address()` instead.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1122>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/272>


### The GC controller (a.k.a. coordinator) is removed

**TL;DR**: The GC controller thread is removed from MMTk core.  The VM binding needs to re-implement
`Collection::spawn_gc_thread`.

API changes:

*   type `GCWorker`
    -   `GCWorker::run`
        +   It now takes ownership of the `Box<GCWorker>` instance instead of borrowing it.
        +   The VM binding can simply call the method `worker.run()` on the worker instance from
            `GCThreadContext::Worker(worker)`.
*   module `mmtk::memory_manager`
    -   `start_worker`
        +   It is now a simple wrapper of `GCWorker::run` for legacy code.  It takes ownership of
            the `Box<GCWorker>` instance, too.

VM bindings need to re-implement the following traits:

*   trait `Collection`
    -   `Collection::spawn_gc_thread`
        +   It no longer asks the binding to spawn the controller thread.
        +   The VM binding can simply remove the code path related to creating the controller
            thread.
        +   Note the API change when calling `GCWorker::run` or `start_worker`.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1067>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/268>

<!--
vim: tw=100
-->
