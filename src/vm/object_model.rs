use crate::plan::CopyContext;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;
use crate::Allocator;
use std::sync::atomic::AtomicU8;

/// VM-specific methods for object model.
///
/// MMTk requires *at least one byte* (or possibly a few bits in certain plans) for MMTk as per-object metadata.
/// MMTk currently assumes the available byte/bits are in the *header words* --
/// this is a strict requirement, and we are looking to revamp the design and support VMs that do not have header words for objects.
///
/// Note that depending on the selected GC plan, only a subset of the methods provided here will be used.
pub trait ObjectModel<VM: VMBinding> {
    /// The offset of the byte available for GC in the header word from the object reference.
    const GC_BYTE_OFFSET: usize;

    /// Get a reference of the GC byte for an object.
    ///
    /// Arguments:
    /// * `o`: The object to get the GC byte from.
    fn get_gc_byte(o: ObjectReference) -> &'static AtomicU8;

    /// Copy an object and return the address of the new object. Usually in the implementation of this method,
    /// `alloc_copy()` and `post_copy()` from a plan's [`CopyContext`](../trait.CopyContext.html) are used for copying.
    ///
    /// Arguments:
    /// * `from`: The address of the object to be copied.
    /// * `allocator`: The allocation semantic to use.
    /// * `copy_context`: The `CopyContext` for the GC thread.
    fn copy(
        from: ObjectReference,
        allocator: Allocator,
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

    /// Return the size required to copy an object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_size_when_copied(object: ObjectReference) -> usize;

    /// Return the alignment requirement for a copy of this object.
    ///
    /// Arguments:
    /// * `object`: The obejct to be queried.
    fn get_align_when_copied(object: ObjectReference) -> usize;

    /// Return the alignment offset requirements for a copy of this object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_align_offset_when_copied(object: ObjectReference) -> isize;

    /// Return the size used by an object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_current_size(object: ObjectReference) -> usize;

    /// Return the object reference for the next object in the heap under
    /// contiguous allocation.
    ///
    /// Arguments:
    /// * `object`: The current object.
    fn get_next_object(object: ObjectReference) -> ObjectReference;

    /// Perform a linear scan and find the next object from a address.
    ///
    /// Arguments:
    /// * `start`: The start address of the object.
    ///
    /// # Safety
    /// We would expect ObjectReferences point to valid objects,
    /// but an arbitrary memory region specified by an address may not reside an object.
    unsafe fn get_object_from_start_address(start: Address) -> ObjectReference;

    /// Return a pointer to the address just past the end of the object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_object_end_address(object: ObjectReference) -> Address;

    /// Get the type descriptor for an object.
    ///
    /// TODO: Do we need this? If so, determine lifetime, return byte[]
    ///
    /// Arguments:
    /// * `reference`: The object to be queried.
    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8];

    /// Return whether the passed object is an array.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn is_array(object: ObjectReference) -> bool;

    /// Return whether the passed object is a primitive array.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn is_primitive_array(object: ObjectReference) -> bool;

    /// Return the array length (in elements). It is undefined behavior if the object is not an array.
    ///
    /// Arguments:
    /// * `object`: The array to be queried.
    fn get_array_length(object: ObjectReference) -> usize;

    // TODO: The methods below should be replaced with a getter of `&AtomicUsize` or `&AtomicU8`,
    // following the pattern of get_gc_byte()

    /// Attempt to set the bits available for memory manager use in an
    /// object. The attempt will only be successful if the current value
    /// of the bits matches `old`. The comparison with the
    /// current value and setting are atomic with respect to other
    /// allocators. Return true if the bits were set, false otherwise.
    ///
    /// Arguments:
    /// * `object`: The object to set bits to.
    /// * `old`: The required current value of the bits.
    /// * `new`: The desired new value of the bits.
    fn attempt_available_bits(object: ObjectReference, old: usize, new: usize) -> bool;

    /// Get the value of bits available for memory manager use in an
    /// object, in preparation for setting those bits.
    ///
    /// Arguments:
    /// * `object`: The object to get bits from.
    fn prepare_available_bits(object: ObjectReference) -> usize;

    /// Set the byte available for memory manager use in an object.
    ///
    /// Arguments:
    /// * `object`: The object to set the byte to.
    /// * `val`: The new value of the byte.
    fn write_available_byte(object: ObjectReference, val: u8);

    /// Read the byte available for memory manager use in an object.
    ///
    /// Arguments:
    /// * `object`: The object to get the byte from.
    fn read_available_byte(object: ObjectReference) -> u8;

    /// Sets the bits available for memory manager use in an object.
    ///
    /// Arguments:
    /// * `object`: The object to set bits to.
    /// * `val`: The new values of the bits.
    fn write_available_bits_word(object: ObjectReference, val: usize);

    /// Read the bits available for memory manager use in an object.
    ///
    /// Arguments:
    /// * `object`: The object to read bits from.
    fn read_available_bits_word(object: ObjectReference) -> usize;

    /// Get the offset of the memory management header from the object
    /// reference address.
    #[deprecated]
    fn gc_header_offset() -> isize;

    /// Return the lowest address of the storage associated with an object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn object_start_ref(object: ObjectReference) -> Address;

    /// Return an address guaranteed to be inside the storage assocatied
    /// with and object.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn ref_to_address(object: ObjectReference) -> Address;

    /// Check if a reference of the given type in another object is
    /// inherently acyclic. The type is given as a TIB.
    ///
    /// Arguments:
    /// * `typeref`: The type of the reference (as a TIB).
    #[deprecated]
    fn is_acyclic(typeref: ObjectReference) -> bool;

    /// Dump debugging information for an object.
    ///
    /// Arguments:
    /// * `object`: The object to be dumped.
    fn dump_object(object: ObjectReference);

    #[deprecated]
    fn get_array_base_offset() -> isize;

    #[deprecated]
    fn array_base_offset_trapdoor<T>(o: T) -> isize;

    #[deprecated]
    fn get_array_length_offset() -> isize;
}
