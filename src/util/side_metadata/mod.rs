//! This is a generic module to work with side metadata (vs. in-object metadata)
//!
//! This module enables the implementation of a wide range of GC algorithms for VMs which do not provide (any/sufficient) in-object space for GC-specific metadata (e.g. marking bits, etc.).
//!
//!
//! # Design
//!
//! MMTk side metadata is designed to be **generic**, and **space-** and **time-** efficient.
//!
//! It aims to support two categories of side metadata:
//!
//! 1. **Global** metadata bits which are per-object metadata and are common to all policies, and
//! 2. **Policy-specific** bits which are only used by certain policies and are not necessarily per-object.
//!
//! To support these categories, MMTk side metadata provides the following features:
//!
//! 1. The granularity of the source data is configurable to $2^n$ bytes, where $n >= 0$.
//! 2. The number of metadata bits per source data unit is configurable to to $2^m$ bits, where $m >= 0$.
//! 3. The total number of metadata bit-sets is only constrained by the amount of available memory.
//! 4. For each metadata bit-set, the metadata space is only allocated for the range of heap addresses it chooses to cover.
//! 5. Bulk-zeroing of metadata bits should be possible. For this, the memory space for each metadata bit-set is contiguous.
//!
//!
//! # How to Use
//!
//! For each side metadata bit-set, first request the metadata bits by calling:
//!
//! ```
//! SideMetadata::request_meta_bits(number_of_bits: usize, align: usize) -> SideMetadataID
//! ```
//!
//! The returned ID is used for future references to this side metadata.
//!
//! Requesting the bits does not allocate any metadata space.
//! So, the next step is to announce the data space you want the metadata to cover, by calling:
//!
//! ```
//! SideMetadata::ensure_meta_space_is_mapped(
//!     start: Address,
//!     size: usize,
//!     metadata_id: SideMetadataID
//! ) -> bool
//! ```
//!
//! On success, this function returns `true`.
//!
//! NOTE-1: A return value of `false` may mean you need to use a different memory space for your data)
//!
//! NOTE-2: As you allocate more memory, you may need to map more metadata space at runtime.
//!
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

mod global;
mod helpers;

pub use global::SideMetadata;
pub use global::SideMetadataID;
