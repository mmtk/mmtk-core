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

`ObjectReference` can no longer represent a NULL reference.

-   API changes:
    -   `ObjectReference::NULL` is removed.
    -   `ObjectReference::is_null()` is removed.
    -   The `target` parameter of the following functions are changed to `Option<ObjectReference>`:
        -   `mmtk::memory_manager::object_reference_write_pre`
        -   `mmtk::memory_manager::object_reference_write_post`
    -   The subsuming barrier `object_reference_write` is labelled as `#[deprecated]`.
    -   `ObjectReference::from_raw_address` now returns `Option<ObjectReference>`.
        -   The new method `ObjectReference::from_raw_address_unchecked` returns `ObjectReference`.
-   VM bindings need to re-implement the following callbacks:
    -   `Edge::load()` now returns `None` if the slot holds NULL or other non-reference values.
    -   Some methods in `ObjectModel` may need to use `ObjectReference::from_raw_address_unchecked`
        to convert non-zero `Address` to `ObjectReference`.  That includes:
        -   `ObjectModel::copy` (non-zero `Address` from `CopyContext::alloc_copy`)
        -   `ObjectModel::get_reference_when_copied_to` (non-zero `Address` from parameter)
        -   `ObjectModel::address_to_ref` (non-zero `Address` from parameter)
    -   `ReferenceGlue::get_referent` now returns `None` if the referent is cleared.
    -   `ReferenceGlue::clear_referent` now needs to be explicitly implemented.
        -   Note: The `Edge` trait does not have a method for storing NULL to a slot.  The VM
            binding needs to implement its own method to store NULL to a slot.
-   Miscellaneous changes:
    -   The following public API functions still return `Option<ObjectReference>`, but VM bindings
        can no longer wrap them into `extern "C"` functions that return `ObjectReference` because
        `ObjectReference::NULL` is removed.  Use `mmtk::util::api_util::NullableObjectReference` for
        the return value.
        -   `memory_manager::get_finalized_object`
        -   `ObjectReference::get_forwarded_object`

*Note: If your VM binding is still using `ReferenceGlue` and the reference processor and
finalization processor in mmtk-core, it is strongly recommended to switch to the
`Scanning::process_weak_refs` method and implement weak reference and finalization processing on the
VM side.*


### Instance methods of `ObjectReference` changed

-   mmtk-core PR: <https://github.com/mmtk/mmtk-core/pull/1122>
-   Binding PR for reference: <https://github.com/mmtk/mmtk-openjdk/pull/272>

Some methods of `ObjectReference` are changed.

-   API changes:
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

The GC controller thread is removed from MMTk core.

-   VM bindings need to re-implement the following callbacks:
    -   `Collection::spawn_gc_thread`:
        -   It no longer asks the binding to spawn the controller thread.
        -   `GCWorker::run` now takes ownership of the `Box<GCWorker>` instance.

<!--
vim: tw=100
-->
