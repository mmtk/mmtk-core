//! This module exposes private items in mmtk-core for testing and benchmarking.
//! They must not be used in production.

use crate::util::metadata::side_metadata::SideMetadataSpec;

use super::Address;

/// Expose `zero_meta_bits` when running `cargo bench`.
pub fn zero_meta_bits(
    meta_start_addr: Address,
    meta_start_bit: u8,
    meta_end_addr: Address,
    meta_end_bit: u8,
) {
    SideMetadataSpec::zero_meta_bits(meta_start_addr, meta_start_bit, meta_end_addr, meta_end_bit)
}

/// Expose `set_meta_bits` when running `cargo bench`.
pub fn set_meta_bits(
    meta_start_addr: Address,
    meta_start_bit: u8,
    meta_end_addr: Address,
    meta_end_bit: u8,
) {
    SideMetadataSpec::set_meta_bits(meta_start_addr, meta_start_bit, meta_end_addr, meta_end_bit)
}
