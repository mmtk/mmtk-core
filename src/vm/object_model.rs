use atomic::Ordering;

use crate::plan::AllocationSemantics;
use crate::plan::CopyContext;
use crate::util::metadata::MetadataSpec;
use crate::util::metadata::{compare_exchange_atomic, metadata_defaults};
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

/// VM-specific methods for object model.
///
/// MMTk does not require but recommands using in-header per-object metadata for better performance.
/// MMTk requires VMs to announce whether they can provide certain per-object metadata in object headers by overriding the metadata related functions in the ObjectModel trait.
///
/// Note that depending on the selected GC plan, only a subset of the methods provided here will be used.
pub trait ObjectModel<VM: VMBinding> {
    fn load_metadata(
        metadata_spec: MetadataSpec,
        object: ObjectReference,
        mask: Option<usize>,
        atomic_ordering: Option<Ordering>,
    ) -> usize {
        let res =
            metadata_defaults::default_load(metadata_spec, object.to_address(), atomic_ordering);
        if let Some(mask) = mask {
            res & mask
        } else {
            res
        }
    }

    fn store_metadata(
        metadata_spec: MetadataSpec,
        object: ObjectReference,
        val: usize,
        mask: Option<usize>,
        atomic_ordering: Option<Ordering>,
    ) {
        if let Some(mask) = mask {
            loop {
                let old_val = metadata_defaults::default_load(
                    metadata_spec,
                    object.to_address(),
                    atomic_ordering,
                );
                let new_val = (old_val & !mask) | (val & mask);
                if compare_exchange_atomic(
                    metadata_spec,
                    object.to_address(),
                    old_val,
                    new_val,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    break;
                }
            }
        } else {
            metadata_defaults::default_store(
                metadata_spec,
                object.to_address(),
                val,
                atomic_ordering,
            )
        }
    }

    fn compare_exchange_metadata(
        metadata_spec: MetadataSpec,
        object: ObjectReference,
        old_val: usize,
        new_val: usize,
        mask: Option<usize>,
        success_order: Ordering,
        failure_order: Ordering,
    ) -> bool {
        debug_assert!(
            mask.is_none() || ((new_val & !mask.unwrap() == 0) && (old_val & !mask.unwrap() == 0)),
            "old_val (0x{:x}) or new_val (0x{:x}) overlap with the mask ({:?})",
            old_val,
            new_val,
            mask
        );

        // if there is a mask, we need to update the new_val according to mask, otherwise we use the input new_val
        let new_val = if let Some(mask) = mask {
            (metadata_defaults::default_load(
                metadata_spec,
                object.to_address(),
                Some(success_order),
            ) & !mask)
                | new_val
        } else {
            new_val
        };
        compare_exchange_atomic(
            metadata_spec,
            object.to_address(),
            old_val,
            new_val,
            success_order,
            failure_order,
        )
    }

    const GLOBAL_LOG_BIT_SPEC: MetadataSpec = metadata_defaults::LOGGING_SIDE_METADATA_SPEC;

    const LOCAL_FORWARDING_POINTER_SPEC: MetadataSpec =
        metadata_defaults::FORWARDING_POINTER_METADATA_SPEC;
    const LOCAL_FORWARDING_BITS_SPEC: MetadataSpec =
        metadata_defaults::FORWARDING_BITS_SIDE_METADATA_SPEC;
    const LOCAL_MARK_BIT_SPEC: MetadataSpec = metadata_defaults::MARKING_SIDE_METADATA_SPEC;
    const LOCAL_LOS_MARK_NURSERY_SPEC: MetadataSpec = metadata_defaults::LOS_SIDE_METADATA_SPEC;
    const LOCAL_UNLOGGED_BIT_SPEC: MetadataSpec = metadata_defaults::UNLOGGED_SIDE_METADATA_SPEC;

    const LAST_GLOBAL_SIDE_METADATA_OFFSET: usize =
        metadata_defaults::LAST_GLOBAL_SIDE_METADATA_OFFSET;

    const LAST_LOCAL_SIDE_METADATA_OFFSET: usize =
        metadata_defaults::LAST_LOCAL_SIDE_METADATA_OFFSET;

    /// Copy an object and return the address of the new object. Usually in the implementation of this method,
    /// `alloc_copy()` and `post_copy()` from a plan's [`CopyContext`](../trait.CopyContext.html) are used for copying.
    ///
    /// Arguments:
    /// * `from`: The address of the object to be copied.
    /// * `semantics`: The allocation semantic to use.
    /// * `copy_context`: The `CopyContext` for the GC thread.
    fn copy(
        from: ObjectReference,
        semantics: AllocationSemantics,
        copy_context: &mut impl CopyContext,
    ) -> ObjectReference;

    /// Copy an object. This is required
    /// for delayed-copy collectors such as compacting collectors. During the
    /// collection, MMTk reserves a region in the heap for an object as per
    /// requirements found from `ObjectModel` and then asks `ObjectModel` to
    /// determine what the object's reference will be post-copy. Return the address
    /// past the end of the copied object.
    ///
    /// Arguments:
    /// * `from`: The address of the object to be copied.
    /// * `to`: The target location.
    /// * `region: The start of the region that was reserved for this object.
    fn copy_to(from: ObjectReference, to: ObjectReference, region: Address) -> Address;

    /// Return the reference that an object will be referred to after it is copied
    /// to the specified region. Used in delayed-copy collectors such as compacting
    /// collectors.
    ///
    /// Arguments:
    /// * `from`: The object to be copied.
    /// * `to`: The region to be copied to.
    fn get_reference_when_copied_to(from: ObjectReference, to: Address) -> ObjectReference;

    /// Return the size used by an object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_current_size(object: ObjectReference) -> usize;

    /// Get the type descriptor for an object.
    ///
    /// FIXME: Do we need this? If so, determine lifetime, return byte[]
    ///
    /// Arguments:
    /// * `reference`: The object to be queried.
    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8];

    /// Return the lowest address of the storage associated with an object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn object_start_ref(object: ObjectReference) -> Address;

    /// Return an address guaranteed to be inside the storage associated
    /// with an object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    // FIXME: this doesn't seem essential. E.g. `get_object_end_address` or `object_start_ref` can cover its functionality.
    fn ref_to_address(object: ObjectReference) -> Address;

    /// Dump debugging information for an object.
    ///
    /// Arguments:
    /// * `object`: The object to be dumped.
    fn dump_object(object: ObjectReference);
}
