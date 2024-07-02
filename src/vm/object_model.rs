use atomic::Ordering;

use self::specs::*;
use crate::util::copy::*;
use crate::util::metadata::header_metadata::HeaderMetadataSpec;
use crate::util::metadata::MetadataValue;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

/// VM-specific methods for object model.
///
/// This trait includes 3 parts:
///
/// 1. Specifications for per object metadata: a binding needs to specify the location for each per object metadata spec.
///    A binding can choose between `in_header()` or `side()`, e.g. `VMGlobalLogBitSpec::side()`.
///    * in_header: a binding needs to specify the bit offset to an object reference that can be used for the per object metadata spec.
///      The actual number of bits required for a spec can be obtained from the `num_bits()` method of the spec type.
///    * side: a binding does not need to provide any specific storage for metadata in the header. Instead, MMTk
///      will use side tables to store the metadata. The following section Side Specs Layout will discuss how to correctly create
///      side metadata specs.
/// 2. In header metadata access: A binding
///    need to further define the functions with suffix _metadata about how to access the bits in the header. We provide default implementations
///    for those methods, assuming the bits in the spec are always available to MMTk. A binding could implement their
///    own routines to access the bits if VM specific treatment is needed (e.g. some bits are not always available to MMTk).
/// 3. VM-specific object info needed by MMTk: MMTk does not know object info as it is VM specific. However, MMTk needs
///    some object information for GC. A binding needs to implement them correctly.
///
/// Note that depending on the selected GC plan, only a subset of the methods provided here will be used.
///
/// # Side Specs Layout
///
/// ## Short version
///
/// * For *global* side metadata:
///   * The first spec: VMGlobalXXXSpec::side_first()
///   * The following specs: VMGlobalXXXSpec::side_after(FIRST_GLOAL.as_spec())
/// * For *local* side metadata:
///   * The first spec: VMLocalXXXSpec::side_first()
///   * The following specs: VMLocalXXXSpec::side_after(FIRST_LOCAL.as_spec())
///
/// ## Detailed explanation
///
/// There are two types of side metadata layout in MMTk:
///
/// 1. Contiguous layout: is the layout in which the whole metadata space for a SideMetadataSpec is contiguous.
/// 2. Chunked layout: is the layout in which the whole metadata memory space, that is shared between MMTk policies, is divided into metadata-chunks. Each metadata-chunk stores all of the metadata for all `SideMetadataSpec`s which apply to a source-data chunk.
///
/// In 64-bits targets, both Global and PolicySpecific side metadata are contiguous.
/// Also, in 32-bits targets, the Global side metadata is contiguous.
/// This means if the starting address (variable named `offset`) of the metadata space for a SideMetadataSpec (`SPEC1`) is `BASE1`, the starting address (`offset`) of the next SideMetadataSpec (`SPEC2`) will be `BASE1 + total_metadata_space_size(SPEC1)`, which is located immediately after the end of the whole metadata space of `SPEC1`.
/// Now, if we add a third SideMetadataSpec (`SPEC3`), its starting address (`offset`) will be `BASE2 + total_metadata_space_size(SPEC2)`, which is located immediately after the end of the whole metadata space of `SPEC2`.
///
/// In 32-bits targets, the PolicySpecific side metadata is chunked.
/// This means for each chunk (2^22 Bytes) of data, which, by definition, is managed by exactly one MMTk policy, there is a metadata chunk (2^22 * some_fixed_ratio Bytes) that contains all of its PolicySpecific metadata.
/// This means if a policy has one SideMetadataSpec (`LS1`), the `offset` of that spec will be `0` (= at the start of a metadata chunk).
/// If there is a second SideMetadataSpec (`LS2`) for this specific policy, the `offset` for that spec will be `0 + required_metadata_space_per_chunk(LS1)`,
/// and for a third SideMetadataSpec (`LS3`), the `offset` will be `BASE(LS2) + required_metadata_space_per_chunk(LS2)`.
///
/// For all other policies, the `offset` starts from zero. This is safe because no two policies ever manage one chunk, so there will be no overlap.
///
/// # Object Layout Addresses
///
/// MMTk tries to be general to cope with different language implementations and different object models. Thus it does not assume the internal of the object model.
/// Instead, MMTk only uses the following addresses for an object. If you find the MMTk's approach does not work for your language in practice, you are welcome to submit an issue
/// or engage with MMTk team on Zulip to disucss further.
///
/// ### Object Reference
///
/// See [`crate::util::address::ObjectReference`]. This is a special address that represents the object.
/// MMTk refers to an object by its object reference. An object reference cannot be NULL, and has to be
/// word aligned ([`crate::util::address::ObjectReference::ALIGNMENT`]). It is allowed that an object
/// reference is not in the allocated memory for the object.
///
/// ### Object Start Address
///
/// The address is returned by an allocation call [`crate::memory_manager::alloc`]. This is the start of the address range of the allocation.
/// [`ObjectModel::ref_to_object_start`] should return this address for a given object.
///
/// ### In-object Address
///
/// As the object reference address may be outside the allocated memory, and calculating the object start address may
/// be complex, MMTk requires a fixed and efficient in-object address for each object. The in-object address should be a constant
/// offset from the object reference address, and should be inside the allocated memory. MMTk requires the conversion
/// from the object reference to the in-object address ([`ObjectModel::ref_to_address`]) and from the in-object address
/// to the object reference ([`ObjectModel::address_to_ref`]).
///
/// ### Object header address
///
/// If a binding allows MMTk to use its header bits for object metadata, they need to supply an object header
/// address ([`ObjectModel::ref_to_header`]). MMTk will access header bits using this address.
pub trait ObjectModel<VM: VMBinding> {
    // Per-object Metadata Spec definitions go here
    //
    // Note a number of Global and PolicySpecific side metadata specifications are already reserved by mmtk-core.
    // Any side metadata offset calculation must consider these to prevent overlaps. A binding should start their
    // side metadata from GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS or LOCAL_SIDE_METADATA_VM_BASE_ADDRESS.

    /// A global 1-bit metadata used by generational plans to track cross-generational pointers. It is generally
    /// located in side metadata.
    ///
    /// Note that for this bit, 0 represents logged (default), and 1 represents unlogged.
    /// This bit is also referred to as unlogged bit in Java MMTk for this reason.
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec;

    /// A local word-size metadata for the forwarding pointer, used by copying plans. It is almost always
    /// located in the object header as it is fine to destroy an object header in order to copy it.
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec;

    /// A local 2-bit metadata for the forwarding status bits, used by copying plans. If your runtime requires
    /// word-aligned addresses (i.e. 4- or 8-bytes), you can use the last two bits in the object header to store
    /// the forwarding bits. Note that you must be careful if you place this in the header as the runtime may
    /// be using those bits for some other reason.
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec;

    /// A local 1-bit metadata for the mark bit, used by most plans that need to mark live objects. Like with the
    /// [forwarding bits](crate::vm::ObjectModel::LOCAL_FORWARDING_BITS_SPEC), you can often steal the last bit in
    /// the object header (due to alignment requirements) for the mark bit. Though some bindings such as the
    /// OpenJDK binding prefer to have the mark bits in side metadata to allow for bulk operations.
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec;

    #[cfg(feature = "object_pinning")]
    /// A local 1-bit metadata specification for the pinning bit, used by plans that need to pin objects. It is
    /// generally in side metadata.
    const LOCAL_PINNING_BIT_SPEC: VMLocalPinningBitSpec;

    /// A local 2-bit metadata used by the large object space to mark objects and set objects as "newly allocated".
    /// Used by any plan with large object allocation. It is generally in the header as we can add an extra word
    /// before the large object to store this metadata. This is fine as the metadata size is insignificant in
    /// comparison to the object size.
    //
    // TODO: Cleanup and place the LOS mark and nursery bits in the header. See here: https://github.com/mmtk/mmtk-core/issues/847
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec;

    /// Set this to true if the VM binding requires the valid object (VO) bits to be available
    /// during tracing. If this constant is set to `false`, it is undefined behavior if the binding
    /// attempts to access VO bits during tracing.
    ///
    /// Note that the VO bits is always available during root scanning even if this flag is false,
    /// which is suitable for using VO bits (and the `is_mmtk_object()` method) for conservative
    /// stack scanning. However, if a binding is also conservative in finding references during
    /// object scanning, they need to set this constant to `true`. See the comments of individual
    /// methods in the `Scanning` trait.
    ///
    /// Depending on the internal implementation of mmtk-core, different strategies for handling
    /// VO bits have different time/space overhead.  mmtk-core will choose the best strategy
    /// according to the configuration of the VM binding, including this flag.  Currently, setting
    /// this flag to true does not impose any additional overhead.
    #[cfg(feature = "vo_bit")]
    const NEED_VO_BITS_DURING_TRACING: bool = false;

    /// A function to non-atomically load the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    ///
    /// # Safety
    /// This is a non-atomic load, thus not thread-safe.
    unsafe fn load_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        mask: Option<T>,
    ) -> T {
        metadata_spec.load::<T>(object.to_header::<VM>(), mask)
    }

    /// A function to atomically load the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `atomic_ordering`: is the atomic ordering for the load operation.
    fn load_metadata_atomic<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        mask: Option<T>,
        ordering: Ordering,
    ) -> T {
        metadata_spec.load_atomic::<T>(object.to_header::<VM>(), mask, ordering)
    }

    /// A function to non-atomically store a value to the specified per-object metadata.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the new metadata value to be stored.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    ///
    /// # Safety
    /// This is a non-atomic store, thus not thread-safe.
    unsafe fn store_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        mask: Option<T>,
    ) {
        metadata_spec.store::<T>(object.to_header::<VM>(), val, mask)
    }

    /// A function to atomically store a value to the specified per-object metadata.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the new metadata value to be stored.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `atomic_ordering`: is the optional atomic ordering for the store operation.
    fn store_metadata_atomic<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        mask: Option<T>,
        ordering: Ordering,
    ) {
        metadata_spec.store_atomic::<T>(object.to_header::<VM>(), val, mask, ordering)
    }

    /// A function to atomically compare-and-exchange the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns `true` if the operation is successful, and `false` otherwise.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `old_val`: is the expected current value of the metadata.
    /// * `new_val`: is the new metadata value to be stored if the compare-and-exchange operation is successful.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `success_order`: is the atomic ordering used if the operation is successful.
    /// * `failure_order`: is the atomic ordering used if the operation fails.
    fn compare_exchange_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        old_val: T,
        new_val: T,
        mask: Option<T>,
        success_order: Ordering,
        failure_order: Ordering,
    ) -> std::result::Result<T, T> {
        metadata_spec.compare_exchange::<T>(
            object.to_header::<VM>(),
            old_val,
            new_val,
            mask,
            success_order,
            failure_order,
        )
    }

    /// A function to atomically perform an add operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// This is a wrapping add.
    /// # Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be added to the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_add_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_add::<T>(object.to_header::<VM>(), val, order)
    }

    /// A function to atomically perform a subtract operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// This is a wrapping sub.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be subtracted from the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_sub_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_sub::<T>(object.to_header::<VM>(), val, order)
    }

    /// A function to atomically perform a bit-and operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-and with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_and_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_and::<T>(object.to_header::<VM>(), val, order)
    }

    /// A function to atomically perform a bit-or operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// Returns the old metadata value.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-or with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    fn fetch_or_metadata<T: MetadataValue>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: T,
        order: Ordering,
    ) -> T {
        metadata_spec.fetch_or::<T>(object.to_header::<VM>(), val, order)
    }

    /// A function to atomically perform an update operation on the specified per-object metadata's content.
    /// The default implementation assumes the bits defined by the spec are always avilable for MMTk to use. If that is not the case, a binding should override this method, and provide their implementation.
    /// The semantics of this method are the same as the `fetch_update()` on Rust atomic types.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is the header metadata spec that tries to perform the operation.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to bit-and with the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    /// # Returns the old metadata value.
    fn fetch_update_metadata<T: MetadataValue, F: FnMut(T) -> Option<T> + Copy>(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        set_order: Ordering,
        fetch_order: Ordering,
        f: F,
    ) -> std::result::Result<T, T> {
        metadata_spec.fetch_update::<T, F>(object.to_header::<VM>(), set_order, fetch_order, f)
    }

    /// Copy an object and return the address of the new object. Usually in the implementation of this method,
    /// `alloc_copy()` and `post_copy()` from [`GCWorkerCopyContext`](util/copy/struct.GCWorkerCopyContext.html)
    /// are used for copying.
    ///
    /// Arguments:
    /// * `from`: The address of the object to be copied.
    /// * `semantics`: The copy semantic to use.
    /// * `copy_context`: The `GCWorkerCopyContext` for the GC thread.
    fn copy(
        from: ObjectReference,
        semantics: CopySemantics,
        copy_context: &mut GCWorkerCopyContext<VM>,
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

    /// Return the size when an object is copied.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_size_when_copied(object: ObjectReference) -> usize;

    /// Return the alignment when an object is copied.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_align_when_copied(object: ObjectReference) -> usize;

    /// Return the alignment offset when an object is copied.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn get_align_offset_when_copied(object: ObjectReference) -> usize;

    /// Get the type descriptor for an object.
    ///
    /// FIXME: Do we need this? If so, determine lifetime, return byte[]
    ///
    /// Arguments:
    /// * `reference`: The object to be queried.
    fn get_type_descriptor(reference: ObjectReference) -> &'static [i8];

    /// This is the worst case expansion that can occur due to object size increasing while
    /// copying. This constant is used to calculate whether a nursery has grown larger than the
    /// mature space for generational plans.
    const VM_WORST_CASE_COPY_EXPANSION: f64 = 1.5;

    /// If this is true, the binding guarantees that an object reference's raw address is always equal to the return value of the `ref_to_address` method
    /// and the return value of the `ref_to_object_start` method. This is a very strong guarantee, but it is also helpful for MMTk to
    /// make some assumptions and optimize for this case.
    /// If a binding sets this to true, and the related methods return inconsistent results, this is an undefined behavior. MMTk may panic
    /// if any assertion catches this error, but may also fail silently.
    const UNIFIED_OBJECT_REFERENCE_ADDRESS: bool = false;

    /// For our allocation result (object_start), the binding may have an offset between the allocation result
    /// and the raw address of their object reference, i.e. object ref's raw address = object_start + offset.
    /// The offset could be zero. The offset is not necessary to be
    /// constant for all the objects. This constant defines the smallest possible offset.
    ///
    /// This is used as an indication for MMTk to predict where object references may point to in some algorithms.
    ///
    /// We should have the invariant:
    /// * object ref >= object_start + OBJECT_REF_OFFSET_LOWER_BOUND
    const OBJECT_REF_OFFSET_LOWER_BOUND: isize;

    /// Return the lowest address of the storage associated with an object. This should be
    /// the address that a binding gets by an allocation call ([`crate::memory_manager::alloc`]).
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn ref_to_object_start(object: ObjectReference) -> Address;

    /// Return the header base address from an object reference. Any object header metadata
    /// in the [`crate::vm::ObjectModel`] declares a piece of header metadata with an offset
    /// from this address. If a binding does not use any header metadata for MMTk, this method
    /// will not be called, and the binding can simply use `unreachable!()` for the method.
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn ref_to_header(object: ObjectReference) -> Address;

    /// Return an address guaranteed to be inside the storage associated
    /// with an object. The returned address needs to be deterministic
    /// for an given object. For a given object, the returned address
    /// *must* be a constant offset from the object reference address.
    ///
    /// Note that MMTk may forge an arbitrary address
    /// directly into a potential object reference, and call this method on the 'object reference'.
    /// In that case, the argument `object` may not be a valid object reference,
    /// and the implementation of this method should not use any object metadata.
    ///
    /// MMTk uses this method more frequently than [`crate::vm::ObjectModel::ref_to_object_start`].
    ///
    /// Arguments:
    /// * `object`: The object to be queried.
    fn ref_to_address(object: ObjectReference) -> Address;

    /// Return an object for a given address returned by `ref_to_address()`.
    /// This does exactly the opposite of `ref_to_address()`. The returned
    /// object reference address *must* be a constant offset from the given address.
    ///
    /// Note that MMTk may forge an address and call this method with the address.
    /// Thus the returned object reference may not always be valid. The binding
    /// should simply apply a constant offset the given address, and return
    /// it as an object reference, and should not assume the returned object reference
    /// is always valid. MMTk is reponsible for using the returned object reference.
    ///
    /// Arguments:
    /// * `addr`: An in-object address.
    fn address_to_ref(addr: Address) -> ObjectReference;

    /// Dump debugging information for an object.
    ///
    /// Arguments:
    /// * `object`: The object to be dumped.
    fn dump_object(object: ObjectReference);

    /// Return if an object is valid from the runtime point of view. This is used
    /// to debug MMTk.
    fn is_object_sane(_object: ObjectReference) -> bool {
        true
    }
}

pub mod specs {
    use crate::util::constants::LOG_BITS_IN_WORD;
    use crate::util::constants::LOG_BYTES_IN_PAGE;
    use crate::util::constants::LOG_MIN_OBJECT_SIZE;
    use crate::util::metadata::side_metadata::*;
    use crate::util::metadata::{
        header_metadata::HeaderMetadataSpec,
        side_metadata::{SideMetadataOffset, SideMetadataSpec},
        MetadataSpec,
    };

    // This macro is invoked in define_vm_metadata_global_spec or define_vm_metadata_local_spec.
    // Use those two to define a new VM metadata spec.
    macro_rules! define_vm_metadata_spec {
        ($(#[$outer:meta])*$spec_name: ident, $is_global: expr, $log_num_bits: expr, $side_min_obj_size: expr) => {
            $(#[$outer])*
            pub struct $spec_name(MetadataSpec);
            impl $spec_name {
                /// The number of bits (in log2) that are needed for the spec.
                pub const LOG_NUM_BITS: usize = $log_num_bits;

                /// Whether this spec is global or local. For side metadata, the binding needs to make sure
                /// global specs are laid out after another global spec, and local specs are laid
                /// out after another local spec. Otherwise, there will be an assertion failure.
                pub const IS_GLOBAL: bool = $is_global;

                /// Declare that the VM uses in-header metadata for this metadata type.
                /// For the specification of the `bit_offset` argument, please refer to
                /// the document of `[crate::util::metadata::header_metadata::HeaderMetadataSpec.bit_offset]`.
                /// The binding needs to make sure that the bits used for a spec in the header do not conflict with
                /// the bits of another spec (unless it is specified that some bits may be reused).
                pub const fn in_header(bit_offset: isize) -> Self {
                    Self(MetadataSpec::InHeader(HeaderMetadataSpec {
                        bit_offset,
                        num_of_bits: 1 << Self::LOG_NUM_BITS,
                    }))
                }

                /// Declare that the VM uses side metadata for this metadata type,
                /// and the side metadata is the first of its kind (global or local).
                /// The first global or local side metadata should be declared with `side_first()`,
                /// and the rest side metadata should be declared with `side_after()` after a defined
                /// side metadata of the same kind (global or local). Logically, all the declarations
                /// create two list of side metadata, one for global, and one for local.
                pub const fn side_first() -> Self {
                    if Self::IS_GLOBAL {
                        Self(MetadataSpec::OnSide(SideMetadataSpec {
                            name: stringify!($spec_name),
                            is_global: Self::IS_GLOBAL,
                            offset: GLOBAL_SIDE_METADATA_VM_BASE_OFFSET,
                            log_num_of_bits: Self::LOG_NUM_BITS,
                            log_bytes_in_region: $side_min_obj_size as usize,
                        }))
                    } else {
                        Self(MetadataSpec::OnSide(SideMetadataSpec {
                            name: stringify!($spec_name),
                            is_global: Self::IS_GLOBAL,
                            offset: LOCAL_SIDE_METADATA_VM_BASE_OFFSET,
                            log_num_of_bits: Self::LOG_NUM_BITS,
                            log_bytes_in_region: $side_min_obj_size as usize,
                        }))
                    }
                }

                /// Declare that the VM uses side metadata for this metadata type,
                /// and the side metadata should be laid out after the given side metadata spec.
                /// The first global or local side metadata should be declared with `side_first()`,
                /// and the rest side metadata should be declared with `side_after()` after a defined
                /// side metadata of the same kind (global or local). Logically, all the declarations
                /// create two list of side metadata, one for global, and one for local.
                pub const fn side_after(spec: &MetadataSpec) -> Self {
                    assert!(spec.is_on_side());
                    let side_spec = spec.extract_side_spec();
                    assert!(side_spec.is_global == Self::IS_GLOBAL);
                    Self(MetadataSpec::OnSide(SideMetadataSpec {
                        name: stringify!($spec_name),
                        is_global: Self::IS_GLOBAL,
                        offset: SideMetadataOffset::layout_after(side_spec),
                        log_num_of_bits: Self::LOG_NUM_BITS,
                        log_bytes_in_region: $side_min_obj_size as usize,
                    }))
                }

                /// Return the inner `[crate::util::metadata::MetadataSpec]` for the metadata type.
                pub const fn as_spec(&self) -> &MetadataSpec {
                    &self.0
                }

                /// Return the number of bits for the metadata type.
                pub const fn num_bits(&self) -> usize {
                    1 << $log_num_bits
                }
            }
            impl std::ops::Deref for $spec_name {
                type Target = MetadataSpec;
                fn deref(&self) -> &Self::Target {
                    self.as_spec()
                }
            }
        };
    }

    // Log bit: 1 bit per object, global
    define_vm_metadata_spec!(
        /// 1-bit global metadata to log an object.
        VMGlobalLogBitSpec,
        true,
        0,
        LOG_MIN_OBJECT_SIZE
    );
    // Forwarding pointer: word size per object, local
    define_vm_metadata_spec!(
        /// 1-word local metadata for spaces that may copy objects.
        /// This metadata has to be stored in the header.
        /// This metadata can be defined at a position within the object payload.
        /// As a forwarding pointer is only stored in dead objects which is not
        /// accessible by the language, it is okay that store a forwarding pointer overwrites object payload
        VMLocalForwardingPointerSpec,
        false,
        LOG_BITS_IN_WORD,
        LOG_MIN_OBJECT_SIZE
    );
    // Forwarding bits: 2 bits per object, local
    define_vm_metadata_spec!(
        /// 2-bit local metadata for spaces that store a forwarding state for objects.
        /// If this spec is defined in the header, it can be defined with a position of the lowest 2 bits in the forwarding pointer.
        VMLocalForwardingBitsSpec,
        false,
        1,
        LOG_MIN_OBJECT_SIZE
    );
    // Mark bit: 1 bit per object, local
    define_vm_metadata_spec!(
        /// 1-bit local metadata for spaces that need to mark an object.
        VMLocalMarkBitSpec,
        false,
        0,
        LOG_MIN_OBJECT_SIZE
    );
    // Pinning bit: 1 bit per object, local
    define_vm_metadata_spec!(
        /// 1-bit local metadata for spaces that support pinning.
        VMLocalPinningBitSpec,
        false,
        0,
        LOG_MIN_OBJECT_SIZE
    );
    // Mark&nursery bits for LOS: 2 bit per page, local
    define_vm_metadata_spec!(
        /// 2-bits local metadata for the large object space. The two bits serve as
        /// the mark bit and the nursery bit.
        VMLocalLOSMarkNurserySpec,
        false,
        1,
        LOG_BYTES_IN_PAGE
    );
}
