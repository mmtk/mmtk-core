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

#### MMTk-side API changes

`ObjectReference` is no longer nullable.  `ObjectReference::NULL` and `ObjectReference::is_null()`
have been removed.

The write barrier APIs changed the type of the `target` parameters from `ObjectReference` to
`Option<ObjectReference>`.  It affects:

-   `mmtk::memory_manager::object_reference_write_pre`
-   `mmtk::memory_manager::object_reference_write_post`

The subsuming barrier `object_reference_write` is labelled as `#[deprecated]` and needs to be
redesigned.  Before a replacement is introduced, VM bindings should use `object_reference_write_pre`
and `object_reference_write_post` instead.

#### Changes required for callbacks implemented by VM bindings

`Edge::load()` now returns `Option<ObjectReference>`.  If the slot is holding a `null` reference or
any non-reference values (such as small integer), it can just return `None`, and MMTk core will skip
that slot.

Several methods in `ObjectModel` are affected by this change because they usually involve converting
`Address` to `ObjectReference`.

-   `ObjectModel::copy` usually calls `copy_context.alloc_copy` to allocate the to-space copy, and
    convert the returned address into an `ObjectReference`.  Since `alloc_copy` never returns 0, you
    may use `ObjectReference::from_raw_address_unchecked`.
-   `get_reference_when_copied_to` never passes 0 to the address parameter, so you may use
    `ObjectReference::from_raw_address_unchecked`, too.
-   `address_to_ref` is the inverse operation of `ref_to_address` which always converts from a
    non-nullable `ObjectReference`.  You may use `ObjectReference::from_raw_address_unchecked`, too.

Some methods of `ReferenceGlue` are affected, too.  *(Note, however, that if your VM binding is
still using `ReferenceGlue` and the reference processor and finalization processor in mmtk-core, it
is strongly recommended to switch to the `Scanning::process_weak_refs` method and implement weak
reference and finalization processing on the VM side.)*

-   `ReferenceGlue::get_referent` now returns `None` to indicate that the referent has been cleared,
    and `ReferenceGlue::is_referent_cleared` is removed.
-   `ReferenceGlue::clear_referent` now needs to be explicitly implemented.  However, if you were
    using `Edge::store` to store NULL reference to the slot, you can no longer do it because
    `Edge::store` now takes a non-nullable `ObjectReference` parameter.  You have to implement your
    own method for storing NULL to a slot.

#### Changes that affect native API

Some public API functions that used to return `Option<ObjectReference>` still return
`Option<ObjectReference>`.  Examples include:

-   `memory_manager::get_finalized_object`
-   `ObjectReference::get_forwarded_object`

However, if you want to expose this API function to native (C/C++) programs, you can no longer
return `ObjectReference::NULL`.  You may use `mmtk::util::api_util::NullableObjectReference` in
`extern "C"` functions to gracefully encode `None` as 0 and pass it to native programs.


### Instance methods of `ObjectReference` changed

-   mmtk-core PR: <https://github.com/mmtk/mmtk-core/pull/1122>
-   Binding PR for reference: <https://github.com/mmtk/mmtk-openjdk/pull/272>

Some instance methods of `ObjectReference` now require a generic argument `<VM: VMBinding>` because
they involve dispatching through the SFT and use the VM-specific `to_address` method under the hood.
These methods include:

-   `is_reachable`
-   `is_live`
-   `is_movable`
-   `get_forwarded_object`
-   `is_in_any_space`
-   `is_sane`

`ObjectReference::value()` is removed.  The user should call `ObjectReference::to_raw_address()` and
then `Address::as_usize()` to get the underlying address as `usize`.


### The GC controller (a.k.a. coordinator) is removed

-   mmtk-core PR: <https://github.com/mmtk/mmtk-core/pull/1067>
-   Binding PR for reference: <https://github.com/mmtk/mmtk-openjdk/pull/268>

The GC controller thread is removed from MMTk core.

`Collection::spawn_gc_thread` will no longer ask the binding to spawn the controller thread.  The
binding should delete the code path for creating the controller thread.

`GCWorker::run` and its wrapper `memory_manager::start_worker` now take ownership of the
`Box<GCWorker>` instance instead of having a `&mut GCWorker` parameter.


<!--
vim: tw=100
-->
