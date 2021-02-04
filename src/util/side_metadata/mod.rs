//! This is a generic module to work with side metadata (vs. in-object metadata)
//!
//! This module enables the implementation of a wide range of GC algorithms for VMs which do not provide (any/sufficient) in-object space for GC-specific metadata (e.g. marking bits, logging bit, etc.).
//!
//!
//! # Design
//!
//! MMTk side metadata is designed to be **generic**, and **space-** and **time-** efficient.
//!
//! It aims to support two categories of side metadata:
//!
//! 1. **Global** metadata bits which are plan-specific but common to all policies, and
//! 2. **Policy-specific** bits which are only used exclusively by certain policies.
//!
//! To support these categories, MMTk side metadata provides the following features:
//!
//! 1. The granularity of the source data (minimum data size) is configurable to $2^n$ bytes, where $n >= 0$.
//! 2. The number of metadata bits per source data unit is configurable to $2^m$ bits, where $m >= 0$.
//! 3. The total number of metadata bit-sets is constrained by the worst-case ratio of global and policy-specific metadata.
//! 4. Metadata space is only allocated on demand.
//! 5. Bulk-zeroing of metadata bits should be possible. For this, the memory space for each metadata bit-set is contiguous per chunk.
//!
//!
//! # How to Use
//!
//! For each global side metadata bit-set, a constant object of the `SideMetadataSpec` struct should be created.
//!
//! For the first global side metadata bit-set:
//!
//! ```
//! const GLOBAL_META_1: SideMetadataSpec = SideMetadataSpec {
//!    scope: SideMetadataScope::Global,
//!    offset: 0,
//!    log_num_of_bits: b,
//!    log_min_obj_size: s,
//! };
//! ```
//!
//! the offset is zero, the number of bits per data is $2^b$, and the minimum object size is $2^s$.
//!
//! Not, to add a second side metadata bit-set, offset needs to be calculated based-on the first global bit-set:
//!
//! ```
//! const GLOBAL_META_2: SideMetadataSpec = SideMetadataSpec {
//!    scope: SideMetadataScope::Global,
//!    offset: meta_bytes_per_chunk(s, b),
//!    log_num_of_bits: bb,
//!    log_min_obj_size: ss,
//! };
//! ```
//!
//! where `meta_bytes_per_chunk` is a const function which calculates the offset based-on `s` and `b` from the first global bit-set.
//!
//! So far, no metadata space is allocated.
//!
//! For this purpose, each plan should override `fn global_side_metadata_per_chuck(&self) -> usize;` to return the size of the global side metadata it needs per chunk. This can be calculated using the `meta_bytes_per_chunk` function.
//!
//! For the local metadata bit-sets, each policy needs to follow the same pattern as the global metadata, with two differences:
//!
//! 1. scope should be `SideMetadataScope::PolicySpecific`,
//! 2. each policy needs to override `fn local_side_metadata_per_chuck(&self) -> usize;
//!
//! After mapping the metadata space, the following operations can be performed on the metadata:
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

mod constants;
mod global;
mod helpers;

pub use global::*;
pub(crate) use helpers::*;
