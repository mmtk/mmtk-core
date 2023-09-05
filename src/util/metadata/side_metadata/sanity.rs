use crate::util::Address;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};
use std::sync::{Mutex, RwLock};

use super::constants::{
    LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO, LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO,
};
use super::{SideMetadataContext, SideMetadataSpec};
#[cfg(target_pointer_width = "64")]
use crate::util::heap::layout::vm_layout::vm_layout;
use crate::util::heap::layout::vm_layout::VMLayout;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout::LOG_BYTES_IN_CHUNK;

/// An internal enum to enhance code style for add/sub
#[cfg(feature = "extreme_assertions")]
enum MathOp {
    Add,
    Sub,
}

/// An internal str used as a name for global side metadata
/// (policy-specific metadata is named after the policy who own it)
static GLOBAL_META_NAME: &str = "Global";

/// This struct includes a hashmap to store the metadata specs information for policy-specific and global metadata for each plan.
/// It uses policy name (or GLOBAL_META_NAME for globals) as the key and keeps a vector of specs as the value.
/// Each plan needs its exclusive instance of this struct to use side metadata specification and content sanity checker.
///
/// NOTE:
/// Content sanity check is expensive and is only activated with the `extreme_assertions` feature.
///
/// FIXME: This struct should be pub(crate) visible, but changing its scope will need changing other scopes, such as the Space trait's. For now, I will not do that.
pub struct SideMetadataSanity {
    specs_sanity_map: HashMap<&'static str, Vec<SideMetadataSpec>>,
}

lazy_static! {
    /// This is a two-level hashmap to store the metadata content for verification purposes.
    /// It keeps a map from side metadata specifications to a second hashmap
    /// which maps data addresses to their current metadata content.
    /// Use u64 to store side metadata values, as u64 is the max length of side metadata we support.
    static ref CONTENT_SANITY_MAP: RwLock<HashMap<SideMetadataSpec, HashMap<Address, u64>>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref SANITY_LOCK: Mutex<()> = Mutex::new(());
}

/// A test helper function which resets contents map to prevent propagation of test failure
#[cfg(test)]
pub(crate) fn reset() {
    CONTENT_SANITY_MAP.write().unwrap().clear()
}

/// Checks whether the input global specifications fit within the current upper bound for all global metadata (limited by `metadata::constants::LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO`).
///
/// Returns `Ok` if all global specs fit and `Err` otherwise.
///
/// Arguments:
/// * `g_specs`: a slice of global specs to be checked.
///
fn verify_global_specs_total_size(g_specs: &[SideMetadataSpec]) -> Result<()> {
    let mut total_size = 0usize;
    for spec in g_specs {
        total_size += super::metadata_address_range_size(spec);
    }

    if total_size
        <= 1usize << (VMLayout::LOG_ARCH_ADDRESS_SPACE - LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO)
    {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Not enough global metadata space for: \n{:?}", g_specs),
        ))
    }
}

/// (For 64-bits targets) Checks whether the input local specifications fit within the current upper bound for each local metadata (limited for each local metadata by `metadata::constants::LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO`).
///
/// Returns `Ok` if all local specs fit and `Err` otherwise.
///
/// Arguments:
/// * `l_specs`: a slice of local specs to be checked.
///
#[cfg(target_pointer_width = "64")]
fn verify_local_specs_size(l_specs: &[SideMetadataSpec]) -> Result<()> {
    for spec in l_specs {
        if super::metadata_address_range_size(spec)
            > 1usize
                << (VMLayout::LOG_ARCH_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO)
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("Local metadata is too big: \n{:?}", spec),
            ));
        }
    }

    Ok(())
}

/// (For 32-bits targets) Checks whether the input local specifications fit within the current upper bound for all chunked local metadata (limited for all chunked local metadata by `metadata::constants::LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO`).
///
/// Returns `Ok` if all local specs fit and `Err` otherwise.
///
/// Arguments:
/// * `l_specs`: a slice of local specs to be checked.
///
#[cfg(target_pointer_width = "32")]
fn verify_local_specs_size(l_specs: &[SideMetadataSpec]) -> Result<()> {
    let mut total_size = 0usize;
    for spec in l_specs {
        total_size +=
            super::metadata_bytes_per_chunk(spec.log_bytes_in_region, spec.log_num_of_bits);
    }

    if total_size > 1usize << (LOG_BYTES_IN_CHUNK - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Not enough local metadata space per chunk for: \n{:?}",
                l_specs
            ),
        ));
    }

    Ok(())
}

/// (For contiguous metadata) Checks whether two input specifications overlap, considering their offsets and maximum size.
///
/// Returns `Err` if overlap is detected.
///
/// Arguments:
/// * `spec_1`: first target specification
/// * `spec_2`: second target specification
///
fn verify_no_overlap_contiguous(
    spec_1: &SideMetadataSpec,
    spec_2: &SideMetadataSpec,
) -> Result<()> {
    let end_1 = spec_1.get_absolute_offset() + super::metadata_address_range_size(spec_1);
    let end_2 = spec_2.get_absolute_offset() + super::metadata_address_range_size(spec_2);

    if !(spec_1.get_absolute_offset() >= end_2 || spec_2.get_absolute_offset() >= end_1) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Overlapping metadata specs detected:\nTHIS:\n{:#?}\nAND:\n{:#?}",
                spec_1, spec_2
            ),
        ));
    }
    Ok(())
}

/// (For chunked metadata) Checks whether two input specifications overlap, considering their offsets and maximum per-chunk size.
///
/// Returns `Err` if overlap is detected.
///
/// Arguments:
/// * `spec_1`: first target specification
/// * `spec_2`: second target specification
///
#[cfg(target_pointer_width = "32")]
fn verify_no_overlap_chunked(spec_1: &SideMetadataSpec, spec_2: &SideMetadataSpec) -> Result<()> {
    let end_1 = spec_1.get_rel_offset()
        + super::metadata_bytes_per_chunk(spec_1.log_bytes_in_region, spec_1.log_num_of_bits);
    let end_2 = spec_2.get_rel_offset()
        + super::metadata_bytes_per_chunk(spec_2.log_bytes_in_region, spec_2.log_num_of_bits);

    if !(spec_1.get_rel_offset() >= end_2 || spec_2.get_rel_offset() >= end_1) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "Overlapping metadata specs detected:\nTHIS:\n{:#?}\nAND:\n{:#?}",
                spec_1, spec_2
            ),
        ));
    }
    Ok(())
}

/// Checks whether a slice of global specifications fit within the memory limits and don't overlap.
///
/// Returns `Ok` if no issue is detected, or otherwise an `Err` explaining the issue.
///
/// Arguments:
/// * `g_specs`: the slice of global specifications to be checked
///
fn verify_global_specs(g_specs: &[SideMetadataSpec]) -> Result<()> {
    verify_global_specs_total_size(g_specs)?;

    for spec_1 in g_specs {
        for spec_2 in g_specs {
            if spec_1 != spec_2 {
                verify_no_overlap_contiguous(spec_1, spec_2)?;
            }
        }
    }

    Ok(())
}

// Clippy likes this
impl Default for SideMetadataSanity {
    fn default() -> Self {
        Self::new()
    }
}

impl SideMetadataSanity {
    /// Creates a new SideMetadataSanity instance.
    pub fn new() -> SideMetadataSanity {
        SideMetadataSanity {
            specs_sanity_map: HashMap::new(),
        }
    }
    /// Returns all global or policy-specific specs based-on the input argument.
    ///
    /// Returns a vector of globals if `global` is true and a vector of locals otherwise.
    ///
    /// Arguments:
    /// * `global`: a boolean to show whether global (`true`) or policy-specific (`false`) specs are required.
    ///
    fn get_all_specs(&self, global: bool) -> Vec<SideMetadataSpec> {
        let mut specs = vec![];
        for (k, v) in self.specs_sanity_map.iter() {
            if !(global ^ (*k == GLOBAL_META_NAME)) {
                specs.append(&mut (*v).clone());
            }
        }

        specs
    }

    /// Verifies that all local side metadata specs:
    /// 1 - are not too big,
    /// 2 - do not overlap.
    ///
    /// Returns `Ok(())` if no issue is detected, or `Err` otherwise.
    ///
    fn verify_local_specs(&self) -> Result<()> {
        let local_specs = self.get_all_specs(false);

        verify_local_specs_size(&local_specs)?;

        for spec_1 in &local_specs {
            for spec_2 in &local_specs {
                if spec_1 != spec_2 {
                    #[cfg(target_pointer_width = "64")]
                    verify_no_overlap_contiguous(spec_1, spec_2)?;
                    #[cfg(target_pointer_width = "32")]
                    verify_no_overlap_chunked(spec_1, spec_2)?;
                }
            }
        }
        Ok(())
    }

    /// An internal method to ensure that a metadata context does not have any issues.
    ///
    /// Arguments:
    /// * `policy_name`: name of the policy of the calling space
    /// * `metadata_context`: the metadata context to examine
    ///
    /// NOTE:
    /// Any unit test using metadata directly or indirectly may need to make sure:
    /// 1 - it uses `util::test_util::serial_test` to prevent metadata sanity conflicts,
    /// 2 - uses exclusive SideMetadata instances (v.s. static instances), and
    /// 3 - uses `util::test_util::with_cleanup` to call `sanity::reset` to cleanup the metadata sanity states to prevent future conflicts.
    ///
    pub(crate) fn verify_metadata_context(
        &mut self,
        policy_name: &'static str,
        metadata_context: &SideMetadataContext,
    ) {
        let mut content_sanity_map = CONTENT_SANITY_MAP.write().unwrap();

        // is this the first call of this function?
        let first_call = !self.specs_sanity_map.contains_key(&GLOBAL_META_NAME);

        if first_call {
            // global metadata combination is the same for all contexts
            verify_global_specs(&metadata_context.global).unwrap();
            self.specs_sanity_map
                .insert(GLOBAL_META_NAME, metadata_context.global.clone());
        } else {
            // make sure the global metadata in the current context has the same length as before
            let g_specs = self.specs_sanity_map.get(&GLOBAL_META_NAME).unwrap();
            assert!(
            g_specs.len() == metadata_context.global.len(),
            "Global metadata must not change between policies! NEW SPECS: {:#?} OLD SPECS: {:#?}",
            metadata_context.global,
            g_specs
        );
        }

        for spec in &metadata_context.global {
            // Make sure all input global specs are actually global
            assert!(
                spec.is_global,
                "Policy-specific spec {:#?} detected in the global specs: {:#?}",
                spec, metadata_context.global
            );
            // On the first call to the function, initialise the content sanity map, and
            // on the future calls, checks the global metadata specs have not changed
            if first_call {
                // initialise the related hashmap
                content_sanity_map.insert(*spec, HashMap::new());
            } else if !self
                .specs_sanity_map
                .get(&GLOBAL_META_NAME)
                .unwrap()
                .contains(spec)
            {
                panic!("Global metadata must not change between policies! NEW SPEC: {:#?} OLD SPECS: {:#?}", spec, self.get_all_specs(true));
            }
        }

        // Is this the first time this function is called by any space of a policy?
        let first_call = !self.specs_sanity_map.contains_key(&policy_name);

        if first_call {
            self.specs_sanity_map
                .insert(policy_name, metadata_context.local.clone());
        }

        for spec in &metadata_context.local {
            // Make sure all input local specs are actually local
            assert!(
                !spec.is_global,
                "Global spec {:#?} detected in the policy-specific specs: {:#?}",
                spec, metadata_context.local
            );
            // The first call from each policy inserts the relevant (spec, hashmap) pair.
            // Future calls only check that the metadata specs have not changed.
            // This should work with multi mmtk instances, because the local side metadata specs are assumed to be constant per policy.
            if first_call {
                // initialise the related hashmap
                content_sanity_map.insert(*spec, HashMap::new());
            } else if !self
                .specs_sanity_map
                .get(policy_name)
                .unwrap()
                .contains(spec)
            {
                panic!(
                    "Policy-specific metadata for -{}- changed from {:#?} to {:#?}",
                    policy_name,
                    self.specs_sanity_map.get(policy_name).unwrap(),
                    metadata_context.local
                )
            }
        }

        self.verify_local_specs().unwrap();
    }

    #[cfg(test)]
    pub fn reset(&mut self) {
        let mut content_sanity_map = CONTENT_SANITY_MAP.write().unwrap();
        self.specs_sanity_map.clear();
        content_sanity_map.clear();
    }
}

/// This verifies two things:
/// 1. Check if data_addr is within the address space that we are supposed to use (LOG_ADDRESS_SPACE). If this fails, we log a warning.
/// 2. Check if metadata address is out of bounds. If this fails, we will panic.
fn verify_metadata_address_bound(spec: &SideMetadataSpec, data_addr: Address) {
    #[cfg(target_pointer_width = "32")]
    assert_eq!(VMLayout::LOG_ARCH_ADDRESS_SPACE, 32, "We assume we use all address space in 32 bits. This seems not true any more, we need a proper check here.");
    #[cfg(target_pointer_width = "32")]
    let data_addr_in_address_space = true;
    #[cfg(target_pointer_width = "64")]
    let data_addr_in_address_space =
        data_addr <= unsafe { Address::from_usize(1usize << vm_layout().log_address_space) };

    if !data_addr_in_address_space {
        warn!(
            "We try get metadata {} for {}, which is not within the address space we should use",
            data_addr, spec.name
        );
    }

    let metadata_addr =
        crate::util::metadata::side_metadata::address_to_meta_address(spec, data_addr);
    let metadata_addr_bound = if spec.is_absolute_offset() {
        spec.upper_bound_address_for_contiguous()
    } else {
        #[cfg(target_pointer_width = "32")]
        {
            spec.upper_bound_address_for_chunked(data_addr)
        }
        #[cfg(target_pointer_width = "64")]
        {
            unreachable!()
        }
    };
    assert!(
        metadata_addr < metadata_addr_bound,
        "We try access metadata address for address {} of spec {} that is not within the bound {}.",
        data_addr,
        spec.name,
        metadata_addr_bound
    );
}

/// Commits a side metadata bulk zero operation.
/// Panics if the metadata spec is not valid.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to perform the bulk zeroing on
/// * `start`: the starting address of the source data
/// * `size`: size of the source data
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_bzero(metadata_spec: &SideMetadataSpec, start: Address, size: usize) {
    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    let start = align_to_region_start(metadata_spec, start);
    let end = align_to_region_start(metadata_spec, start + size);
    match sanity_map.get_mut(metadata_spec) {
        Some(spec_sanity_map) => {
            // zero entries where the key (data_addr) is in the range (start, start+size)
            for (k, v) in spec_sanity_map.iter_mut() {
                // If the source address is in the bzero's range
                if *k >= start && *k < end {
                    *v = 0;
                }
            }
        }
        None => {
            panic!("Invalid Metadata Spec: {}", metadata_spec.name);
        }
    }
}

/// Commits a side metadata bulk set operation (set the related bits to all 1s).
/// Panics if the metadata spec is not valid.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to perform the bulk set on
/// * `start`: the starting address of the source data
/// * `size`: size of the source data
#[cfg(feature = "extreme_assertions")]
pub fn verify_bset(metadata_spec: &SideMetadataSpec, start: Address, size: usize) {
    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    let start = align_to_region_start(metadata_spec, start);
    let end = align_to_region_start(metadata_spec, start + size);
    let max_value = (1 << (1 << metadata_spec.log_num_of_bits)) - 1;
    match sanity_map.get_mut(metadata_spec) {
        Some(spec_sanity_map) => {
            let mut cursor = start;
            let step: usize = 1 << metadata_spec.log_bytes_in_region;
            while cursor < end {
                spec_sanity_map.insert(cursor, max_value);
                cursor += step;
            }
        }
        None => {
            panic!("Invalid Metadata Spec!");
        }
    }
}

/// Commits a side metadata bulk copy operation
/// (set the bits to the corresponding bits of another metadata).
/// Panics if the metadata spec is not valid.
///
/// The source and destination metadata must have the same granularity.
///
/// Arguments:
/// * `dst_spec`: the metadata spec to bulk copy to
/// * `start`: the starting address of the data
/// * `size`: size of the data
/// * `src_spec`: the metadata spec to bulk copy from
#[cfg(feature = "extreme_assertions")]
pub fn verify_bcopy(
    dst_spec: &SideMetadataSpec,
    start: Address,
    size: usize,
    src_spec: &SideMetadataSpec,
) {
    assert_eq!(src_spec.log_num_of_bits, dst_spec.log_num_of_bits);
    assert_eq!(src_spec.log_bytes_in_region, dst_spec.log_bytes_in_region);

    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    let start = align_to_region_start(dst_spec, start);
    let end = align_to_region_start(dst_spec, start + size);

    // Rust doesn't like mutably borrowing two entries from `sanity_map` at the same time.
    // So we load all values from `sanity_map[src_spec]` into an intermediate HashMap,
    // and then store them to `sanity_map[dst_spec]`.

    let mut tmp_map = HashMap::new();

    {
        let src_map = sanity_map
            .get_mut(src_spec)
            .expect("Invalid source Metadata Spec!");

        let mut cursor = start;
        let step: usize = 1 << src_spec.log_bytes_in_region;
        while cursor < end {
            let src_value = src_map.get(&cursor).copied().unwrap_or(0u64);
            tmp_map.insert(cursor, src_value);
            cursor += step;
        }
    }
    {
        let dst_map = sanity_map
            .get_mut(dst_spec)
            .expect("Invalid destination Metadata Spec!");

        let mut cursor = start;
        let step: usize = 1 << dst_spec.log_bytes_in_region;
        while cursor < end {
            let src_value = tmp_map.get(&cursor).copied().unwrap();
            dst_map.insert(cursor, src_value);
            cursor += step;
        }
    }
}

#[cfg(feature = "extreme_assertions")]
use crate::util::metadata::metadata_val_traits::*;

#[cfg(feature = "extreme_assertions")]
fn truncate_value<T: MetadataValue>(log_num_of_bits: usize, val: u64) -> u64 {
    // truncate the val if metadata's bits is fewer than the type's bits
    if log_num_of_bits < T::LOG2 as usize {
        val & ((1 << (1 << log_num_of_bits)) - 1)
    } else {
        val
    }
}

#[cfg(feature = "extreme_assertions")]
#[cfg(test)]
mod truncate_tests {
    use super::*;

    #[test]
    fn test_truncate() {
        assert_eq!(truncate_value::<u8>(2, 0), 0);
        assert_eq!(truncate_value::<u8>(2, 15), 15);
        assert_eq!(truncate_value::<u8>(2, 16), 0);
        assert_eq!(truncate_value::<u8>(2, 17), 1);
    }
}

// When storing a value for a data address, we align the data address to the region start.
// So when accessing any data address in the region, we will use the same data address to fetch the metadata value.
fn align_to_region_start(spec: &SideMetadataSpec, data_addr: Address) -> Address {
    data_addr.align_down(1 << spec.log_bytes_in_region)
}

/// Ensures a side metadata load operation returns the correct side metadata content.
/// Panics if:
/// 1 - the metadata spec is not valid,
/// 2 - data address is not valid,
/// 3 - the loaded side metadata content is not equal to the correct content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to verify the loaded content for
/// * `data_addr`: the address of the source data
/// * `actual_val`: the actual content returned by the side metadata load operation
#[cfg(feature = "extreme_assertions")]
pub fn verify_load<T: MetadataValue>(
    metadata_spec: &SideMetadataSpec,
    data_addr: Address,
    actual_val: T,
) {
    let data_addr = align_to_region_start(metadata_spec, data_addr);
    let actual_val: u64 = actual_val.to_u64().unwrap();
    verify_metadata_address_bound(metadata_spec, data_addr);
    let sanity_map = &mut CONTENT_SANITY_MAP.read().unwrap();
    match sanity_map.get(metadata_spec) {
        Some(spec_sanity_map) => {
            // A content of None is Ok because we may load before store
            let expected_val = if let Some(expected_val) = spec_sanity_map.get(&data_addr) {
                *expected_val
            } else {
                0u64
            };
            assert!(
                expected_val == actual_val,
                "verify_load({:#?}, {}) -> Expected (0x{:x}) but found (0x{:x})",
                metadata_spec,
                data_addr,
                expected_val,
                actual_val
            );
        }
        None => panic!("Invalid Metadata Spec: {:#?}", metadata_spec),
    }
}

/// Commits a side metadata store operation.
/// Panics if:
/// 1 - the loaded side metadata content is not equal to the correct content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to commit the store operation for
/// * `data_addr`: the address of the source data
/// * `metadata`: the metadata content to store
#[cfg(feature = "extreme_assertions")]
pub fn verify_store<T: MetadataValue>(
    metadata_spec: &SideMetadataSpec,
    data_addr: Address,
    metadata: T,
) {
    let data_addr = align_to_region_start(metadata_spec, data_addr);
    let metadata: u64 = metadata.to_u64().unwrap();
    verify_metadata_address_bound(metadata_spec, data_addr);
    let new_val_wrapped = truncate_value::<T>(metadata_spec.log_num_of_bits, metadata);
    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    match sanity_map.get_mut(metadata_spec) {
        Some(spec_sanity_map) => {
            // Newly mapped memory including the side metadata memory is zeroed
            let content = spec_sanity_map.entry(data_addr).or_insert(0);
            *content = new_val_wrapped;
        }
        None => panic!("Invalid Metadata Spec: {:#?}", metadata_spec),
    }
}

/// Commits an update operation and ensures it returns the correct old side metadata content.
/// Panics if:
/// 1 - the metadata spec is not valid,
/// 2 - the old side metadata content is not equal to the correct old content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to verify the old content for
/// * `data_addr`: the address of the source data
/// * `old_val`: the expected old value
/// * `new_val`: the new value the metadata should hold.
#[cfg(feature = "extreme_assertions")]
pub fn verify_update<T: MetadataValue>(
    metadata_spec: &SideMetadataSpec,
    data_addr: Address,
    old_val: T,
    new_val: T,
) {
    let data_addr = align_to_region_start(metadata_spec, data_addr);
    verify_metadata_address_bound(metadata_spec, data_addr);

    // truncate the new_val if metadata's bits is fewer than the type's bits
    let new_val_wrapped =
        truncate_value::<T>(metadata_spec.log_num_of_bits, new_val.to_u64().unwrap());
    println!(
        "verify_update old = {} new = {} wrapped = {:x}",
        old_val, new_val, new_val_wrapped
    );

    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    match sanity_map.get_mut(metadata_spec) {
        Some(spec_sanity_map) => {
            let cur_val = spec_sanity_map.entry(data_addr).or_insert(0);
            assert_eq!(
                old_val.to_u64().unwrap(),
                *cur_val,
                "Expected old value: {} but found {}",
                old_val,
                cur_val
            );
            *cur_val = new_val_wrapped;
        }
        None => panic!("Invalid metadata spec: {:#?}", metadata_spec),
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    #[test]
    fn test_side_metadata_sanity_verify_global_specs_total_size() {
        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: true,
            offset: SideMetadataOffset::addr(Address::ZERO),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };
        let spec_2 = SideMetadataSpec {
            name: "spec_2",
            is_global: true,
            offset: SideMetadataOffset::layout_after(&spec_1),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_global_specs_total_size(&[spec_1]).is_ok());
        #[cfg(target_pointer_width = "64")]
        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_ok());
        #[cfg(target_pointer_width = "32")]
        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_err());

        let spec_2 = SideMetadataSpec {
            name: "spec_2",
            is_global: true,
            offset: SideMetadataOffset::layout_after(&spec_1),
            log_num_of_bits: 3,
            log_bytes_in_region: 1,
        };

        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_err());

        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: true,
            offset: SideMetadataOffset::addr(Address::ZERO),
            log_num_of_bits: 1,
            #[cfg(target_pointer_width = "64")]
            log_bytes_in_region: 0,
            #[cfg(target_pointer_width = "32")]
            log_bytes_in_region: 2,
        };
        let spec_2 = SideMetadataSpec {
            name: "spec_2",
            is_global: true,
            offset: SideMetadataOffset::layout_after(&spec_1),
            log_num_of_bits: 3,
            #[cfg(target_pointer_width = "64")]
            log_bytes_in_region: 2,
            #[cfg(target_pointer_width = "32")]
            log_bytes_in_region: 4,
        };

        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_ok());
        assert!(verify_global_specs_total_size(&[spec_1, spec_2, spec_1]).is_err());
    }

    #[test]
    fn test_side_metadata_sanity_verify_no_overlap_contiguous() {
        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: true,
            offset: SideMetadataOffset::addr(Address::ZERO),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };
        let spec_2 = SideMetadataSpec {
            name: "spec_2",
            is_global: true,
            offset: SideMetadataOffset::layout_after(&spec_1),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_no_overlap_contiguous(&spec_1, &spec_1).is_err());
        assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_ok());

        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: true,
            offset: SideMetadataOffset::addr(unsafe { Address::from_usize(1) }),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_err());

        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: true,
            offset: SideMetadataOffset::addr(Address::ZERO),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };
        let spec_2 = SideMetadataSpec {
            name: "spec_2",
            is_global: true,
            // We specifically make up an invalid offset
            offset: SideMetadataOffset::addr(
                spec_1.get_absolute_offset() + metadata_address_range_size(&spec_1) - 1,
            ),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_err());
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn test_side_metadata_sanity_verify_no_overlap_chunked() {
        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: false,
            offset: SideMetadataOffset::rel(0),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };
        let spec_2 = SideMetadataSpec {
            name: "spec_2",
            is_global: false,
            offset: SideMetadataOffset::layout_after(&spec_1),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_no_overlap_chunked(&spec_1, &spec_1).is_err());
        assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_ok());

        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: false,
            offset: SideMetadataOffset::rel(1),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_err());

        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: false,
            offset: SideMetadataOffset::rel(0),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };
        let spec_2 = SideMetadataSpec {
            name: "spec_2",
            is_global: false,
            // We make up an invalid offset
            offset: SideMetadataOffset::rel(
                spec_1.get_rel_offset()
                    + metadata_bytes_per_chunk(spec_1.log_bytes_in_region, spec_1.log_num_of_bits)
                    - 1,
            ),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_err());
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn test_side_metadata_sanity_verify_local_specs_size() {
        let spec_1 = SideMetadataSpec {
            name: "spec_1",
            is_global: false,
            offset: SideMetadataOffset::rel(0),
            log_num_of_bits: 0,
            log_bytes_in_region: 0,
        };

        assert!(verify_local_specs_size(&[spec_1]).is_ok());
        assert!(verify_local_specs_size(&[spec_1, spec_1]).is_err());
        assert!(verify_local_specs_size(&[spec_1, spec_1, spec_1, spec_1, spec_1]).is_err());
    }
}
