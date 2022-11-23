//! This is a generic module to work with metadata including side metadata and in-object metadata.
//!
//! This module is designed to enable the implementation of a wide range of GC algorithms for VMs with various combinations of in-object and on-side space for GC-specific metadata (e.g. forwarding bits, marking bit, logging bit, etc.).
//!
//! The new metadata design differentiates per-object metadata (e.g. forwarding-bits and marking-bit) from other types of metadata including per-address (e.g. alloc-bit) and per-X (where X != object size), because the per-object metadata can optionally be kept in the object headers.
//!
//! MMTk acknowledges the VM-dependant nature of the in-object metadata, and asks the VM bindings to contribute by implementing the related parts in the ['ObjectModel'](crate::vm::ObjectModel).
//!
//!
//! # Side Metadata
//!
//! ## Design
//!
//! MMTk side metadata is designed to be **generic**, and **space-** and **time-** efficient.
//!
//! It aims to support two categories of metadata:
//!
//! 1. **Global** metadata bits which are plan-specific but common to all policies, and
//! 2. **Policy-specific** bits which are only used exclusively by certain policies.
//!
//! To support these categories, MMTk metadata provides the following features:
//!
//! 1. The granularity of the source data (minimum data size) is configurable to $2^n$ bytes, where $n >= 0$.
//! 2. The number of metadata bits per source data unit is configurable to $2^m$ bits, where $m >= 0$.
//! 3. The total number of metadata bit-sets is constrained by the worst-case ratio of global and policy-specific metadata.
//! 4. Metadata space is only allocated on demand.
//! 5. Bulk-zeroing of metadata bits should be possible. For this, the memory space for each metadata bit-set is contiguous per chunk.
//!
//! ### 64-bits targets
//!
//!‌ In 64-bits targets, each MMTk side metadata bit-set is organized as a contiguous space.
//! The base address for both the global and the local side metadata are constants (e.g. `GLOBAL_SIDE_METADATA_BASE_ADDRESS` and `LOCAL_SIDE_METADATA_BASE_ADDRESS`).
//!
//! In this case, a schematic of the local and global side metadata looks like:
//!
//!     _______________________________ <= global-1 = GLOBAL_SIDE_METADATA_BASE_ADDRESS
//!     |                             |
//!     |        Global-1             |
//!     |_____________________________| <= global-2 = global-1 +
//!     |                             |                 metadata_address_range_size(global-1)
//!     |        Global-2             |
//!     |                             |
//!     |_____________________________| <= global-3 = global-2 +
//!     |                             |                 metadata_address_range_size(global-2)
//!     |        Not Mapped           |
//!     |                             |
//!     |_____________________________| <= global-end = GLOBAL_SIDE_METADATA_BASE_ADDRESS +
//!     |                             |         MAX_HEAP_SIZE * Global_WCR
//!     |                             |
//!     |                             |
//!     |_____________________________| <= local-1 = LOCAL_SIDE_METADATA_BASE_ADDRESS
//!     |                             |
//!     |      PolicySpecific-1       |
//!     |                             |
//!     |_____________________________| <= local-2 = local-1 + metadata_address_range_size(local-1)
//!     |                             |
//!     |      PolicySpecific-2       |
//!     |                             |
//!     |_____________________________| <= local-3 = local-2 + metadata_address_range_size(local-2)
//!     |                             |
//!     |         Not Mapped          |
//!     |                             |
//!     |                             |
//!     |_____________________________| <= local-end = LOCAL_SIDE_METADATA_BASE_ADDRESS +
//!                                             MAX_HEAP_SIZE * PolicySpecific_WCR
//!‌
//!‌ ### 32-bits targets
//!
//! In 32-bits targets, the global side metadata is organized the same way as 64-bits, but the policy-specific side metadata is organized per chunk of data (each chunk is managed exclusively by one policy).
//! This means, when a new chunk is mapped, the policy-specific side metadata for the whole chunk is also mapped.
//!
//! In this case, a schematic of the local and global side metadata looks like:
//!
//!     _______________________________ <= global-1 = GLOBAL_SIDE_METADATA_BASE_ADDRESS(e.g. 0x1000_0000)
//!     |                             |
//!     |        Global-1             |
//!     |_____________________________| <= global-2 = global-1 +
//!     |                             |                 metadata_address_range_size(global-1)
//!     |        Global-2             |
//!     |                             |
//!     |_____________________________| <= global-3 = global-2 +
//!     |                             |                 metadata_address_range_size(global-2)
//!     |        Not Mapped           |
//!     |                             |
//!     |_____________________________| <= global-end = GLOBAL_SIDE_METADATA_BASE_ADDRESS +
//!     |                             |         MAX_HEAP_SIZE * Global_WCR
//!     |                             |
//!     |                             |
//!     |_____________________________| <= LOCAL_SIDE_METADATA_BASE_ADDRESS
//!     |                             |
//!     |      PolicySpecific         |
//!     |                             |
//!     |                             |
//!     |                             |
//!     |_____________________________| <= local-end = LOCAL_SIDE_METADATA_BASE_ADDRESS +
//!                                             MAX_HEAP_SIZE * PolicySpecific_WCR
//!
//!
//!‌ And inside the PolicySpecific space, each per chunk policy-specific side metadata looks like:
//!
//!     _______________________________     <= offset-1 = 0x0
//!     |                             |
//!     |        Local-1              |
//!     |_____________________________|     <= offset-2 = metadata_bytes_per_chunk(Local-1)
//!     |                             |
//!     |        Local-2              |
//!     |                             |
//!     |_____________________________|     <= offset-g3 = offset-g2 + metadata_bytes_per_chunk(Local-2)
//!     |                             |
//!     |        Not Mapped           |
//!     |                             |
//!     |_____________________________|     <= 4MB * PolicySpecific_WCR
//!
//!
//!
//!
//! # How to Use
//!
//! ## Declare metadata specs
//!
//! For each global metadata bit-set, a constant instance of the `MetadataSpec` struct should be created.
//!
//! If the metadata is per-object and may possibly reside in objects, the constant instance should be created in the VM's ObjectModel.
//! For instance, the forwarding-bits metadata spec should be assigned to `LOCAL_FORWARDING_BITS_SPEC` in [`ObjectModel`](crate::vm::ObjectModel).
//! The VM binding decides whether to put these metadata bit-sets in-objects or on-side.
//!
//! For other metadata bit-sets, constant `MetadataSpec` instances, created inside MMTk by plans/policies, are used in conjunction with the access functions from the current module.
//!
//! Example:
//!
//! For the first global side metadata bit-set:
//!
//! ```
//! const GLOBAL_META_1: MetadataSpec = MetadataSpec {
//!    is_side_metadata: true,
//!    is_global: true,
//!    offset: GLOBAL_SIDE_METADATA_BASE_ADDRESS,
//!    log_num_of_bits: b1,
//!    log_bytes_in_region: s1,
//! };
//! ```
//!
//! Here, the number of bits per data is $2^b1$, and the minimum object size is $2^s1$.
//! The `offset` is actually the base address for a global side metadata bit-set.
//! For the first bit-set, `offset` is `GLOBAL_SIDE_METADATA_BASE_ADDRESS`.
//!
//! Now, to add a second side metadata bit-set, offset needs to be calculated based-on the first global bit-set:
//!
//! ```
//! const GLOBAL_META_2: MetadataSpec = MetadataSpec {
//!    is_side_metadata: true,
//!     is_global: true,
//!    offset: GLOBAL_META_1.offset + metadata_address_range_size(GLOBAL_META_1)
//!    log_num_of_bits: b2,
//!    log_bytes_in_region: s2,
//! };
//! ```
//!
//! where `metadata_address_range_size` is a const function which calculates the total metadata space size of a contiguous side metadata bit-set based-on `s` and `b`.
//!
//! The policy-specific side metadata for 64-bits targets, and the global side metadata for 32-bits targets are used on the same way, except that their base addresses are different.
//!
//! Policy-specific side metadata for 32-bits target is slightly different, because it is chunk-based.
//!
//! For the first local side metadata bit-set:
//!
//! ```
//! const LOCAL_META_1: MetadataSpec = MetadataSpec {
//!    is_side_metadata: true,
//!    is_global: false,
//!    offset: 0,
//!    log_num_of_bits: b1,
//!    log_bytes_in_region: s1,
//! };
//! ```
//!
//! Here, the `offset` is actually the inter-chunk offset of the side metadata from the start of the current side metadata chunk.
//!
//! Now, to add a second side metadata bit-set, offset needs to be calculated based-on the first global bit-set:
//!
//! ```
//! const LOCAL_META_2: MetadataSpec = MetadataSpec {
//!    is_side_metadata: true,
//!    is_global: false,
//!    offset: LOCAL_META_1.offset + metadata_bytes_per_chunk(LOCAL_META_1)
//!    log_num_of_bits: b2,
//!    log_bytes_in_region: s2,
//! };
//! ```
//!
//! So far, we declared each metadata specs.
//! We can now use the in-object metadata through the access functions in the VM bindings ObjectModel.
//! For side metadata, the next step is to allocate metadata space.
//!
//!
//! ## Create and allocate side metadata for spaces
//!
//! A space needs to know all global metadata specs and its own policy-specific/local metadata specs in order to calculate and allocate metadata space.
//! When a space is created by a plan (e.g. SemiSpace::new), the plan can create its global specs by `MetadataContext::new_global_specs(&[GLOBAL_META_1, GLOBAL_META_2])`. Then,
//! the global specs are passed to each space that the plan creates.
//!
//! Each space will then combine the global specs and its own local specs to create a [SideMetadataContext](crate::util::metadata::side_metadata::SideMetadataContext).
//! Allocating side metadata space and accounting its memory usage is done by `SideMetadata`. If a space uses `CommonSpace`, `CommonSpace` will create `SideMetadata` and manage
//! reserving and allocating metadata space when necessary. If a space does not use `CommonSpace`, it should create `SideMetadata` itself and manage allocating metadata space
//! as its own responsibility.
//!
//! ## Access side metadata
//!
//! After mapping the metadata space, the following operations can be performed with a specific metadata spec:
//!
//! 1. atomic load
//! 2. atomic store
//! 3. atomic compare-and-exchange
//! 4. atomic fetch-and-add
//! 5. atomic fetch-and-sub
//! 6. load (non-atomic)
//! 7. store (non-atomic)
//! 8. bulk zeroing
//!

mod global;
pub mod header_metadata;
mod metadata_val_traits;
pub mod side_metadata;
pub use metadata_val_traits::*;

pub(crate) mod log_bit;
pub(crate) mod pin_bit;

pub use global::*;
