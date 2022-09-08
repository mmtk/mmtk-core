use atomic::Ordering;

use self::specs::*;
use crate::util::metadata::header_metadata::HeaderMetadataSpec;
use crate::util::{Address, ObjectReference};
use crate::vm::VMBinding;

use crate::util::copy::*;

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
///    need to further define the functions with suffix _metadata about how to access the bits in the header. A binding may use
///    functions in the [`header_metadata`] module if the bits are always available to MMTk, or they could implement their
///    own routines to access the bits if VM specific treatment is needed (e.g. some bits are not always available to MMTk).
/// 3. VM-specific object info needed by MMTk: MMTk does not know object info as it is VM specific. However, MMTk needs
///    some object information for GC. A binding needs to implement them correctly.
///
/// Note that depending on the selected GC plan, only a subset of the methods provided here will be used.
///
/// Side Specs Layout
///
/// Short version
///
/// * For *global* side metadata:
///   * The first spec: VMGlobalXXXSpec::side_first()
///   * The following specs: VMGlobalXXXSpec::side_after(FIRST_GLOAL.as_spec())
/// * For *local* side metadata:
///   * The first spec: VMLocalXXXSpec::side_first()
///   * The following specs: VMLocalXXXSpec::side_after(FIRST_LOCAL.as_spec())
///
/// Detailed explanation
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
/// [`HeaderMetadataSpec`]: ../util/metadata/header_metadata/struct.HeaderMetadataSpec.html
/// [`SideMetadataSpec`]:   ../util/metadata/side_metadata/strutc.SideMetadataSpec.html
/// [`header_metadata`]:    ../util/metadata/header_metadata/index.html
/// [`GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS`]: ../util/metadata/side_metadata/constant.GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS.html
/// [`LOCAL_SIDE_METADATA_VM_BASE_ADDRESS`]:  ../util/metadata/side_metadata/constant.LOCAL_SIDE_METADATA_VM_BASE_ADDRESS.html
pub trait ObjectModel<VM: VMBinding> {
    // Per-object Metadata Spec definitions go here
    //
    // Note a number of Global and PolicySpecific side metadata specifications are already reserved by mmtk-core.
    // Any side metadata offset calculation must consider these to prevent overlaps. A binding should start their
    // side metadata from GLOBAL_SIDE_METADATA_VM_BASE_ADDRESS or LOCAL_SIDE_METADATA_VM_BASE_ADDRESS.

    /// The metadata specification of the global log bit. 1 bit.
    /// Note that for this bit, 0 represents logged (default), and 1 represents unlogged.
    /// This bit is also referred to as unlogged bit in Java MMTk for this reason.
    const GLOBAL_LOG_BIT_SPEC: VMGlobalLogBitSpec;

    /// The metadata specification for the forwarding pointer, used by copying plans. Word size.
    const LOCAL_FORWARDING_POINTER_SPEC: VMLocalForwardingPointerSpec;
    /// The metadata specification for the forwarding status bits, used by copying plans. 2 bits.
    const LOCAL_FORWARDING_BITS_SPEC: VMLocalForwardingBitsSpec;
    /// The metadata specification for the mark bit, used by most plans that need to mark live objects. 1 bit.
    const LOCAL_MARK_BIT_SPEC: VMLocalMarkBitSpec;
    /// The metadata specification for the mark-and-nursery bits, used by most plans that has large object allocation. 2 bits.
    const LOCAL_LOS_MARK_NURSERY_SPEC: VMLocalLOSMarkNurserySpec;

    /// A function to load the specified per-object metadata's content.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
    /// * `object`: is a reference to the target object.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `atomic_ordering`: is an optional atomic ordering for the load operation. An input value of `None` means the load operation is not atomic, and an input value of `Some(Ordering::X)` means the atomic load operation will use the `Ordering::X`.
    ///
    /// # Returns the metadata value as a word. If the metadata size is less than a word, the effective value is stored in the low-order bits of the word.
    ///
    fn load_metadata(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        mask: Option<usize>,
        atomic_ordering: Option<Ordering>,
    ) -> usize;

    /// A function to store a value to the specified per-object metadata.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the new metadata value to be stored.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `atomic_ordering`: is an optional atomic ordering for the store operation. An input value of `None` means the store operation is not atomic, and an input value of `Some(Ordering::X)` means the atomic store operation will use the `Ordering::X`.
    ///
    fn store_metadata(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: usize,
        mask: Option<usize>,
        atomic_ordering: Option<Ordering>,
    );

    /// A function to atomically compare-and-exchange the specified per-object metadata's content.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
    /// * `object`: is a reference to the target object.
    /// * `old_val`: is the expected current value of the metadata.
    /// * `new_val`: is the new metadata value to be stored if the compare-and-exchange operation is successful.
    /// * `mask`: is an optional mask value for the metadata. This value is used in cases like the forwarding pointer metadata, where some of the bits are reused by other metadata such as the forwarding bits.
    /// * `success_order`: is the atomic ordering used if the operation is successful.
    /// * `failure_order`: is the atomic ordering used if the operation fails.
    ///
    /// # Returns `true` if the operation is successful, and `false` otherwise.
    ///
    fn compare_exchange_metadata(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        old_val: usize,
        new_val: usize,
        mask: Option<usize>,
        success_order: Ordering,
        failure_order: Ordering,
    ) -> bool;

    /// A function to atomically perform an add operation on the specified per-object metadata's content.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be added to the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    /// # Returns the old metadata value as a word.
    ///
    fn fetch_add_metadata(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: usize,
        order: Ordering,
    ) -> usize;

    /// A function to atomically perform a subtract operation on the specified per-object metadata's content.
    ///
    /// # Arguments:
    ///
    /// * `metadata_spec`: is one of the const `MetadataSpec` instances from the ObjectModel trait, for the target metadata. Whether the metadata is in-header or on-side is a VM-specific choice.
    /// * `object`: is a reference to the target object.
    /// * `val`: is the value to be subtracted from the current value of the metadata.
    /// * `order`: is the atomic ordering of the fetch-and-add operation.
    ///
    /// # Returns the old metadata value as a word.
    ///
    fn fetch_sub_metadata(
        metadata_spec: &HeaderMetadataSpec,
        object: ObjectReference,
        val: usize,
        order: Ordering,
    ) -> usize;

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
    fn get_align_offset_when_copied(object: ObjectReference) -> isize;

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

    /// For our allocation result `[cell, cell + bytes)`, if a binding's
    /// definition of `ObjectReference` may point outside the cell (i.e. `object_ref >= cell + bytes`),
    /// the binding needs to provide a `Some` value for this constant and
    /// the value is the maximum of `object_ref - cell`. If a binding's
    /// `ObjectReference` always points to an address in the cell (i.e. `[cell, cell + bytes)`),
    /// they can leave this as `None`.
    /// MMTk allocators use this value to make sure that the metadata for object reference is properly set.
    const OBJECT_REF_OFFSET_BEYOND_CELL: Option<usize> = None;

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
        ($spec_name: ident, $is_global: expr, $log_num_bits: expr, $side_min_obj_size: expr) => {
            pub struct $spec_name(MetadataSpec);
            impl $spec_name {
                pub const LOG_NUM_BITS: usize = $log_num_bits;
                pub const IS_GLOBAL: bool = $is_global;
                pub const fn in_header(bit_offset: isize) -> Self {
                    Self(MetadataSpec::InHeader(HeaderMetadataSpec {
                        bit_offset,
                        num_of_bits: 1 << Self::LOG_NUM_BITS,
                    }))
                }
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
                pub const fn side_after(spec: &MetadataSpec) -> Self {
                    debug_assert!(spec.is_on_side());
                    let side_spec = spec.extract_side_spec();
                    debug_assert!(side_spec.is_global == Self::IS_GLOBAL);
                    Self(MetadataSpec::OnSide(SideMetadataSpec {
                        name: stringify!($spec_name),
                        is_global: Self::IS_GLOBAL,
                        offset: SideMetadataOffset::layout_after(side_spec),
                        log_num_of_bits: Self::LOG_NUM_BITS,
                        log_bytes_in_region: $side_min_obj_size as usize,
                    }))
                }
                #[inline(always)]
                pub const fn as_spec(&self) -> &MetadataSpec {
                    &self.0
                }
                pub const fn num_bits(&self) -> usize {
                    1 << $log_num_bits
                }
            }
            impl std::ops::Deref for $spec_name {
                type Target = MetadataSpec;
                #[inline(always)]
                fn deref(&self) -> &Self::Target {
                    self.as_spec()
                }
            }
        };
    }

    // Log bit: 1 bit per object, global
    define_vm_metadata_spec!(VMGlobalLogBitSpec, true, 0, LOG_MIN_OBJECT_SIZE);
    // Forwarding pointer: word size per object, local
    define_vm_metadata_spec!(
        VMLocalForwardingPointerSpec,
        false,
        LOG_BITS_IN_WORD,
        LOG_MIN_OBJECT_SIZE
    );
    // Forwarding bits: 2 bits per object, local
    define_vm_metadata_spec!(VMLocalForwardingBitsSpec, false, 1, LOG_MIN_OBJECT_SIZE);
    // Mark bit: 1 bit per object, local
    define_vm_metadata_spec!(VMLocalMarkBitSpec, false, 0, LOG_MIN_OBJECT_SIZE);
    // Mark&nursery bits for LOS: 2 bit per page, local
    define_vm_metadata_spec!(VMLocalLOSMarkNurserySpec, false, 1, LOG_BYTES_IN_PAGE);
}
