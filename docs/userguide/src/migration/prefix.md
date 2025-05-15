# API Migration Guide

This document lists changes to the MMTk-VM API that require changes to be made by the VM bindings.
VM binding developers can use this document as a guide to update their code to maintain
compatibility with the latest release of MMTk.


## View control

Choose how many details you want to read.

{{#include ../../assets/snippets/view-controls.html}}

<!--

Notes for the mmtk-core developers:

-   Make sure you add to the **upcoming release**.  Check the current version in `Cargo.toml`.
-   You may add new items or edit existing items before a release, whichever makes sense.
-   No need to mention API changes that are source compatible and do not require VM binding code
    to be updated.
-   Use the [template](template.md).
-   100 characters per line.  Those who read this doc in text editors and IDEs will thank you.
    -   vim: "gq" formats the selected lines, and "gqap" formats one paragraph.
    -   vscode: The "Rewrap" plugin can re-wrap a paragraph with one hot key.

-->

<div id="api-migration-detail-body"><!-- We use JavaScript to process things within this div. -->

<!-- Insert new versions here -->

## 0.32.0

### `Options` no longer differentiates between environment variables and command line arguments.

```admonish tldr
We replaced both `Options::set_from_command_line` and `Options::set_from_env_var` with
`Options::set_from_string` because all options can now be set via either environment variable or
command line arguments.
```

API changes:

-   module `util::options`
    +   `Options::set_from_command_line`: Removed.
        *   Use `Options::set_from_string` instead.
    +   `Options::set_from_env_var`: Removed.
        *   Use `Options::set_from_string` instead.
    +   `Options::set_bulk_from_command_line`: Removed.
        *   Use `Options::set_bulk_from_string` instead.
    +   All `<T>` in `MMTKOption<T>` now must implement `FromStr`.
        *   This means you can parse a string into `T` when setting an `MMTKOption<T>`.  For
            example, `options.plan.set(user_input.parse()?);`.

## 0.30.0

### `live_bytes_in_last_gc` becomes a runtime option, and returns a map for live bytes in each space

```admonish tldr
`count_live_bytes_in_gc` is now a runtime option instead of a features (build-time), and we collect
live bytes statistics per space. Correspondingly, `memory_manager::live_bytes_in_last_gc` now returns a map for
live bytes in each space.
```

API changes:

-   module `util::options`
    +   `Options` includes `count_live_bytes_in_gc`, which defaults to `false`. This can be turned on at run-time.
    +   The old `count_live_bytes_in_gc` feature is removed.
-   module `memory_manager`
    +   `live_bytes_in_last_gc` now returns a `HashMap<&'static str, LiveBytesStats>`. The keys are
        strings for space names, and the values are statistics for live bytes in the space.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1238>


### mmap-related functions require annotation

```admonish tldr
Memory-mapping functions in `mmtk::util::memory` now take an additional `MmapAnnotation` argument.
```

API changes:

-   module `util::memory`
    +   The following functions take an additional `MmapAnnotation` argument.
        *   `dzmmap`
        *   `dzmmap_noreplace`
        *   `mmap_noreserve`

## 0.28.0

### `handle_user_collection_request` returns `bool`

```admonish tldr
`memory_manager::handle_user_collection_request` now returns a boolean value to indicate whether a GC
is triggered by the method or not. Bindings may use the return value to do some post-gc cleanup, or
simply ignore the return value.
```

API changes:

-   module `memory_manager`
    +   `handle_user_collection_request` now returns `bool` to indicate if a GC is triggered by the method.
        Bindings may use the value, or simply ignore it.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/issues/1205>
-   Examples:
    + https://github.com/mmtk/mmtk-julia/pull/177: Ignore return value.


### `ObjectReference` must point inside an object

```admonish tldr
`ObjectReference` is now required to be an address within an object.  The concept of "in-object
address" and related methods are removed.  Some methods which used to depend on the "in-object
address" no longer need the `<VM>` type argument.
```

API changes:

-   struct `ObjectReference`
    +   Its "raw address" must be within an object now.
    +   The following methods which were used to access the in-object address are removed.
        *   `from_address`
        *   `to_address`
        *   When accessing side metadata, the "raw address" should be used, instead.
    +   The following methods no longer have the `<VM>` type argument.
        *   `get_forwarded_object`
        *   `is_in_any_space`
        *   `is_live`
        *   `is_movable`
        *   `is_reachable`
-   module `memory_manager`
    +   `is_mmtk_object`: It now requires the address parameter to be non-zero and word-aligned.
        *   Otherwise it will not be a legal `ObjectReference` in the first place.  The user should
            filter out such illegal values.
    +   The following functions no longer have the `<VM>` type argument.
        *   `find_object_from_internal_pointer`
        *   `is_in_mmtk_space`
        *   `is_live_object`
        *   `is_pinned`
        *   `pin_object`
        *   `unpin_object`
-   struct `Region`
    +   The following methods no longer have the `<VM>` type argument.
        *   `containing`
-   trait `ObjectModel`
    +   `IN_OBJECT_ADDRESS_OFFSET`: removed because it is no longer needed.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/issues/1170>
-   Examples:
    +   https://github.com/mmtk/mmtk-openjdk/pull/286: a simple case
    +   https://github.com/mmtk/mmtk-jikesrvm/issues/178: a VM that needs much change for this

## 0.27.0

### `is_mmtk_object` returns `Option<ObjectReference>`

```admonish tldr
`memory_manager::is_mmtk_object` now returns `Option<ObjectReference>` instead of `bool`.
Bindings can use the returned object reference instead of computing the object reference at the binding side.
```

API changes:
* module `memory_manager`
  - `is_mmtk_object` now returns `Option<ObjectReference>`.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1165>
-   Example: <https://github.com/mmtk/mmtk-ruby/pull/86>

### Introduce `ObjectModel::IN_OBJECT_ADDRESS_OFFSET`

```admonish tldr
We used to have `ObjectModel::ref_to_address` and `ObjectModel::address_to_ref`, and require
the object reference and the in-object address to have a constant offset. Now, the two methods
are removed, and replaced with a constant `ObjectModel::IN_OBJECT_ADDRESS_OFFSET`.
```

API changes:
* trait `ObjectModel`
  - The methods `ref_to_address` and `address_to_ref` are removed.
  - Users are required to specify `IN_OBJECT_ADDRESS_OFFSET` instead, which is the offset from the object
    reference to the in-object address (the in-object address was the return value for the old `ref_to_address()`).
* type `ObjectReference`
  - Add a constant `ALIGNMENT` which equals to the word size. All object references should be at least aligned
    to the word size. This is checked in debug builds when an `ObjectReference` is constructed.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1159>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/283>

## 0.26.0

### Rename "edge" to "slot"

```admonish tldr
The word "edge" **in many identifiers** have been changed to "slot" if it actaully means slot.
Notable items include the traits `Edge`, `EdgeVisitor`, the module `edge_shape`, and member types
and functions in the `Scanning` and `VMBinding` traits.  The VM bindings should not only make
changes in response to the changes in MMTk-core, but also make changes to their own identifiers if
they also use "edge" where it should have been "slot".  The find/replace tools in text editors and
the refactoring/renaming tools in IDEs should be helpful.
```

API changes:

*   module `edge_shape` -> `slot`
*   type `RootsWorkFactory`
    -   `<ES: Edge>` -> `<SL: Slot>`
    -   `create_process_edge_roots_work` -> `create_process_roots_work`
*   type `SimpleEdge` -> `SimpleSlot`
*   type `UnimplementedMemorySliceEdgeIterator` -> `UnimplementedMemorySliceSlotIterator`
*   trait `Edge` -> `Slot`
*   trait `EdgeVisitor` -> `SlotVisitor`
    -   `<ES: Edge>` -> `<SL: Slot>`
    -   `visit_edge` -> `visit_slot`
*   trait `MemorySlice`
    -   `Edge` -> `SlotType`
    -   `EdgeIterator` -> `SlotIterator`
    -   `iter_edges` -> `iter_slots`
*   trait `Scanning`
    -   `support_edge_enqueuing` -> `support_slot_enqueuing`
    -   `scan_object`
        +   `<EV: EdgeVisitor>` -> `<SV: SlotVisitor>`
    -   `scan_roots_in_mutator_thread`
        +   Type parameter of `factory` changed. See type `RootsWorkFactory`.
    -   `scan_vm_specific_roots`
        +   Same as above.
*   trait `VMBinding`
    -   `VMEdge` -> `VMSlot`

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1134>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/274>


## 0.25.0

### `ObjectReference` is no longer nullable

```admonish tldr
`ObjectReference` can no longer represent a NULL reference.  Some methods of `ObjectReference` and
the write barrier functions in `memory_manager` are changed.  VM bindings need to re-implement
methods of the `Edge`, `ObjectModel` and `ReferenceGlue` traits.
```

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

Not API change, but worth noting:

*   Functions that return `Option<ObjectReference>`
    -   `memory_manager::get_finalized_object`
    -   `ObjectReference::get_forwarded_object`
        +   The functions listed above did not change, as they still return
            `Option<ObjectReference>`.  But some VM bindings used to expose them to native programs
            by wrapping them into `extern "C"` functions that return `ObjectReference`, and return
            `ObjectReference::NULL` for `None`.  This is no longer possible since we removed
            `ObjectReference::NULL`.  The VM bindings should use
            `mmtk::util::api_util::NullableObjectReference` for the return type instead.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1064>
-   PR: <https://github.com/mmtk/mmtk-core/pull/1130> (for write barriers)
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/265>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/273> (for write barriers)


### Instance methods of `ObjectReference` changed

```admonish tldr
Some methods of `ObjectReference` now have a type parameter `<VM>`.  `ObjectReference::value()` is
removed.
```

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

```admonish tldr
The GC controller thread is removed from MMTk core.  The VM binding needs to re-implement
`Collection::spawn_gc_thread` and call `GCWorker::run` differently.
```

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
*   trait `Collection`
    -   `Collection::spawn_gc_thread`
        +   It no longer asks the binding to spawn the controller thread.
        +   The VM binding can simply remove the code path related to creating the controller
            thread.
        +   Note the API change when calling `GCWorker::run` or `start_worker`.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/1067>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/268>

</div>

<script type="text/javascript">
const isApiMigrationGuide = true;
</script>

<!--
vim: tw=100
-->
