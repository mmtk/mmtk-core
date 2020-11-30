use crate::plan::CopyContext;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::AllocationSemantics;

/// VM-specific methods for object model.
///
/// MMTk requires *at least one byte* (or possibly a few bits in certain plans) for MMTk as per-object metadata.
/// MMTk currently assumes the available byte/bits are in the *header words* --
/// this is a strict requirement, and we are looking to revamp the design and support VMs that do not have header words for objects.
///
/// Note that depending on the selected GC plan, only a subset of the methods provided here will be used.
pub trait ObjectModel<VM: VMBinding> {
    /// Whether an exclusive GC byte in each object's header word is available for MMTk.
    /// If such a byte is not available in the VM, MMTk will handle it in its own memory.
    ///
    /// Note: Currently only the `true` value is supported.
    const HAS_GC_BYTE: bool = true;
    /// The offset of the GC byte from the object reference, in number of bytes.
    ///
    /// Notes:
    ///  - This value is only effective when `HAS_GC_BYTE` is set to `true`.
    ///  - It is recommanded that GC byte is the low-order byte of the word that contains it. \
    /// E.g. in a 64-bits little endian system, the recommanded offset is `8*K`.
    ///
    const GC_BYTE_OFFSET: isize = 0;

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
