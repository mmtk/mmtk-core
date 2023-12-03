//! Side Specs Layout
//!
//! Short version
//!
//! * For *global* side metadata:
//!   * The first spec: VMGlobalXXXSpec::side_first()
//!   * The following specs: VMGlobalXXXSpec::side_after(FIRST_GLOAL.as_spec())
//! * For *local* side metadata:
//!   * The first spec: VMLocalXXXSpec::side_first()
//!   * The following specs: VMLocalXXXSpec::side_after(FIRST_LOCAL.as_spec())
//!
//! Detailed explanation
//!
//! There are two types of side metadata layout in MMTk:
//!
//! 1. Contiguous layout: is the layout in which the whole metadata space for a SideMetadataSpec is contiguous.
//! 2. Chunked layout: is the layout in which the whole metadata memory space, that is shared between MMTk policies, is divided into metadata-chunks. Each metadata-chunk stores all of the metadata for all `SideMetadataSpec`s which apply to a source-data chunk.
//!
//! In 64-bits targets, both Global and PolicySpecific side metadata are contiguous.
//! Also, in 32-bits targets, the Global side metadata is contiguous.
//! This means if the starting address (variable named `offset`) of the metadata space for a SideMetadataSpec (`SPEC1`) is `BASE1`, the starting address (`offset`) of the next SideMetadataSpec (`SPEC2`) will be `BASE1 + total_metadata_space_size(SPEC1)`, which is located immediately after the end of the whole metadata space of `SPEC1`.
//! Now, if we add a third SideMetadataSpec (`SPEC3`), its starting address (`offset`) will be `BASE2 + total_metadata_space_size(SPEC2)`, which is located immediately after the end of the whole metadata space of `SPEC2`.
//!
//! In 32-bits targets, the PolicySpecific side metadata is chunked.
//! This means for each chunk (2^22 Bytes) of data, which, by definition, is managed by exactly one MMTk policy, there is a metadata chunk (2^22 * some_fixed_ratio Bytes) that contains all of its PolicySpecific metadata.
//! This means if a policy has one SideMetadataSpec (`LS1`), the `offset` of that spec will be `0` (= at the start of a metadata chunk).
//! If there is a second SideMetadataSpec (`LS2`) for this specific policy, the `offset` for that spec will be `0 + required_metadata_space_per_chunk(LS1)`,
//! and for a third SideMetadataSpec (`LS3`), the `offset` will be `BASE(LS2) + required_metadata_space_per_chunk(LS2)`.
//!
//! For all other policies, the `offset` starts from zero. This is safe because no two policies ever manage one chunk, so there will be no overlap.

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
