//! This module exposes private items in mmtk-core for testing and benchmarking. They must not be
//! used in production.
//!
//! # Notes on inlining
//!
//! In mmtk-core, we refrain from inserting inlining hints manually.  But we use `#[inline(always)]`
//! in this module explicitly because the functions here are simple wrappers of private functions,
//! and the compiler usually fails to make the right decision given that those functions are not
//! used often, and we don't compile the benchmarks using feedback-directed optimizations.

use crate::util::metadata::side_metadata::SideMetadataSpec;

use super::Address;

/// Expose `zero_meta_bits` when running `cargo bench`.
#[inline(always)]
pub fn zero_meta_bits(
    meta_start_addr: Address,
    meta_start_bit: u8,
    meta_end_addr: Address,
    meta_end_bit: u8,
) {
    SideMetadataSpec::zero_meta_bits(meta_start_addr, meta_start_bit, meta_end_addr, meta_end_bit)
}

/// Expose `set_meta_bits` when running `cargo bench`.
#[inline(always)]
pub fn set_meta_bits(
    meta_start_addr: Address,
    meta_start_bit: u8,
    meta_end_addr: Address,
    meta_end_bit: u8,
) {
    SideMetadataSpec::set_meta_bits(meta_start_addr, meta_start_bit, meta_end_addr, meta_end_bit)
}
