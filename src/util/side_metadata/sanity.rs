use crate::util::Address;
use std::collections::HashMap;
use std::io::{Error, ErrorKind, Result};
use std::sync::{Mutex, RwLock};

use super::constants::LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO;
use super::constants::LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO;
use super::{SideMetadataContext, SideMetadataSpec};
use crate::util::heap::layout::vm_layout_constants::LOG_ADDRESS_SPACE;
#[cfg(target_pointer_width = "32")]
use crate::util::heap::layout::vm_layout_constants::LOG_BYTES_IN_CHUNK;

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
    static ref CONTENT_SANITY_MAP: RwLock<HashMap<SideMetadataSpec, HashMap<Address, usize>>> =
        RwLock::new(HashMap::new());
    pub(crate) static ref SANITY_LOCK: Mutex<()> = Mutex::new(());
}

/// A test helper function which resets contents map to prevent propagation of test failure
#[cfg(test)]
pub(crate) fn reset() {
    CONTENT_SANITY_MAP.write().unwrap().clear()
}

/// Checks whether the input global specifications fit within the current upper bound for all global metadata (limited by `side_metadata::constants::LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO`).
///
/// Returns `Ok` if all global specs fit and `Err` otherwise.
///
/// Arguments:
/// * `g_specs`: a slice of global specs to be checked.
///
fn verify_global_specs_total_size(g_specs: &[SideMetadataSpec]) -> Result<()> {
    let mut total_size = 0usize;
    for spec in g_specs {
        total_size += super::metadata_address_range_size(*spec);
    }

    if total_size <= 1usize << (LOG_ADDRESS_SPACE - LOG_GLOBAL_SIDE_METADATA_WORST_CASE_RATIO) {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Not enough global metadata space for: \n{:?}", g_specs),
        ))
    }
}

/// (For 64-bits targets) Checks whether the input local specifications fit within the current upper bound for each local metadata (limited for each local metadata by `side_metadata::constants::LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO`).
///
/// Returns `Ok` if all local specs fit and `Err` otherwise.
///
/// Arguments:
/// * `l_specs`: a slice of local specs to be checked.
///
#[cfg(target_pointer_width = "64")]
fn verify_local_specs_size(l_specs: &[SideMetadataSpec]) -> Result<()> {
    for spec in l_specs {
        if super::metadata_address_range_size(*spec)
            > 1usize << (LOG_ADDRESS_SPACE - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO)
        {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("Local metadata is too big: \n{:?}", spec),
            ));
        }
    }

    Ok(())
}

/// (For 32-bits targets) Checks whether the input local specifications fit within the current upper bound for all chunked local metadata (limited for all chunked local metadata by `side_metadata::constants::LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO`).
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
        total_size += super::meta_bytes_per_chunk(spec.log_min_obj_size, spec.log_num_of_bits);
    }

    if total_size > 1usize << (LOG_BYTES_IN_CHUNK - LOG_LOCAL_SIDE_METADATA_WORST_CASE_RATIO) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Not local metadata space per chunk for: \n{:?}", l_specs),
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
    let end_1 = spec_1.offset + super::metadata_address_range_size(*spec_1);
    let end_2 = spec_2.offset + super::metadata_address_range_size(*spec_2);

    if !(spec_1.offset >= end_2 || spec_2.offset >= end_1) {
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
    let end_1 = spec_1.offset
        + super::meta_bytes_per_chunk(spec_1.log_min_obj_size, spec_1.log_num_of_bits);
    let end_2 = spec_2.offset
        + super::meta_bytes_per_chunk(spec_2.log_min_obj_size, spec_2.log_num_of_bits);

    if !(spec_1.offset >= end_2 || spec_2.offset >= end_1) {
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
    let v = verify_global_specs_total_size(g_specs);
    if v.is_err() {
        return v;
    }

    for spec_1 in g_specs {
        for spec_2 in g_specs {
            if spec_1 != spec_2 {
                let v = verify_no_overlap_contiguous(spec_1, spec_2);
                if v.is_err() {
                    return v;
                }
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

        let v = verify_local_specs_size(&local_specs);
        if v.is_err() {
            return v;
        }

        for spec_1 in &local_specs {
            for spec_2 in &local_specs {
                if spec_1 != spec_2 {
                    #[cfg(target_pointer_width = "64")]
                    let v = verify_no_overlap_contiguous(spec_1, spec_2);
                    #[cfg(target_pointer_width = "32")]
                    let v = verify_no_overlap_chunked(spec_1, spec_2);
                    if v.is_err() {
                        return v;
                    }
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
            if !spec.scope.is_global() {
                panic!(
                    "Policy-specific spec {:#?} detected in the global specs: {:#?}",
                    spec, metadata_context.global
                );
            }
            // On the first call to the function, initialise the content sanity map, and
            // on the future calls, checks the global metadata specs have not changed
            if first_call {
                // initialise the related hashmap
                content_sanity_map.insert(*spec, HashMap::new());
            } else if !self
                .specs_sanity_map
                .get(&GLOBAL_META_NAME)
                .unwrap()
                .contains(&spec)
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
            if spec.scope.is_global() {
                panic!(
                    "Global spec {:#?} detected in the policy-specific specs: {:#?}",
                    spec, metadata_context.local
                );
            }
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
                .contains(&spec)
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

/// Commits a side metadata bulk zero operation.
/// Panics if the metadata spec is not valid.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to perform the bulk zeroing on
/// * `start`: the starting address of the source data
/// * `size`: size of the source data
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_bzero(metadata_spec: SideMetadataSpec, start: Address, size: usize) {
    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    match sanity_map.get_mut(&metadata_spec) {
        Some(spec_sanity_map) => {
            // zero entries where the key (data_addr) is in the range (start, start+size)
            for (k, v) in spec_sanity_map.iter_mut() {
                // If the source address is in the bzero's range
                if *k >= start && *k < start + size {
                    *v = 0;
                }
            }
        }
        None => {
            panic!("Invalid Metadata Spec!");
        }
    }
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
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_load(metadata_spec: &SideMetadataSpec, data_addr: Address, actual_val: usize) {
    let sanity_map = &mut CONTENT_SANITY_MAP.read().unwrap();
    match sanity_map.get(&metadata_spec) {
        Some(spec_sanity_map) => {
            // A content of None is Ok because we may load before store
            let expected_val = if let Some(expected_val) = spec_sanity_map.get(&data_addr) {
                *expected_val
            } else {
                0usize
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
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_store(metadata_spec: SideMetadataSpec, data_addr: Address, metadata: usize) {
    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    match sanity_map.get_mut(&metadata_spec) {
        Some(spec_sanity_map) => {
            // Newly mapped memory including the side metadata memory is zeroed
            let content = spec_sanity_map.entry(data_addr).or_insert(0);
            *content = metadata;
        }
        None => panic!("Invalid Metadata Spec: {:#?}", metadata_spec),
    }
}

/// A helper function encapsulating the common parts of addition and subtraction
#[cfg(feature = "extreme_assertions")]
fn do_math(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
    val: usize,
    math_op: MathOp,
) -> Result<usize> {
    let sanity_map = &mut CONTENT_SANITY_MAP.write().unwrap();
    match sanity_map.get_mut(&metadata_spec) {
        Some(spec_sanity_map) => {
            // Newly mapped memory including the side metadata memory is zeroed
            let cur_val = spec_sanity_map.entry(data_addr).or_insert(0);
            let old_val = *cur_val;
            match math_op {
                MathOp::Add => *cur_val += val,
                MathOp::Sub => *cur_val -= val,
            }
            Ok(old_val)
        }
        None => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("Invalid Metadata Spec: {:#?}", metadata_spec),
        )),
    }
}

/// Commits a fetch and add operation and ensures it returns the correct old side metadata content.
/// Panics if:
/// 1 - the metadata spec is not valid,
/// 2 - the old side metadata content is not equal to the correct old content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to verify the old content for
/// * `data_addr`: the address of the source data
/// * `val_to_add`: the number to be added to the old content
/// * `actual_old_val`: the actual old content returned by the side metadata fetch and add operation
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_add(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
    val_to_add: usize,
    actual_old_val: usize,
) {
    match do_math(metadata_spec, data_addr, val_to_add, MathOp::Add) {
        Ok(expected_old_val) => {
            assert!(
                actual_old_val == expected_old_val,
                "Expected (0x{:x}) but found (0x{:x})",
                expected_old_val,
                actual_old_val
            );
        }
        Err(e) => panic!("{}", e),
    }
}

/// Commits a fetch and sub operation and ensures it returns the correct old side metadata content.
/// Panics if:
/// 1 - the metadata spec is not valid,
/// 2 - the old side metadata content is not equal to the correct old content.
///
/// Arguments:
/// * `metadata_spec`: the metadata spec to verify the old content for
/// * `data_addr`: the address of the source data
/// * `val_to_sub`: the number to be subtracted from the old content
/// * `actual_old_val`: the actual old content returned by the side metadata fetch and sub operation
///
#[cfg(feature = "extreme_assertions")]
pub fn verify_sub(
    metadata_spec: SideMetadataSpec,
    data_addr: Address,
    val_to_sub: usize,
    actual_old_val: usize,
) {
    match do_math(metadata_spec, data_addr, val_to_sub, MathOp::Sub) {
        Ok(expected_old_val) => {
            assert!(
                actual_old_val == expected_old_val,
                "Expected (0x{:x}) but found (0x{:x})",
                expected_old_val,
                actual_old_val
            );
        }
        Err(e) => panic!("{}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    #[test]
    fn test_side_metadata_sanity_verify_global_specs_total_size() {
        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };
        let spec_2 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: metadata_address_range_size(spec_1),
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_global_specs_total_size(&[spec_1]).is_ok());
        #[cfg(target_pointer_width = "64")]
        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_ok());
        #[cfg(target_pointer_width = "32")]
        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_err());

        let spec_2 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: metadata_address_range_size(spec_1),
            log_min_obj_size: 1,
            log_num_of_bits: 3,
        };

        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_err());

        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            #[cfg(target_pointer_width = "64")]
            log_min_obj_size: 0,
            #[cfg(target_pointer_width = "32")]
            log_min_obj_size: 2,
            log_num_of_bits: 1,
        };
        let spec_2 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: metadata_address_range_size(spec_1),
            #[cfg(target_pointer_width = "64")]
            log_min_obj_size: 2,
            #[cfg(target_pointer_width = "32")]
            log_min_obj_size: 4,
            log_num_of_bits: 3,
        };

        assert!(verify_global_specs_total_size(&[spec_1, spec_2]).is_ok());
        assert!(verify_global_specs_total_size(&[spec_1, spec_2, spec_1]).is_err());
    }

    #[test]
    fn test_side_metadata_sanity_verify_no_overlap_contiguous() {
        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };
        let spec_2 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: metadata_address_range_size(spec_1),
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_no_overlap_contiguous(&spec_1, &spec_1).is_err());
        assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_ok());

        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 1,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_err());

        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };
        let spec_2 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: metadata_address_range_size(spec_1) - 1,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_no_overlap_contiguous(&spec_1, &spec_2).is_err());
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn test_side_metadata_sanity_verify_no_overlap_chunked() {
        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };
        let spec_2 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: meta_bytes_per_chunk(spec_1.log_min_obj_size, spec_1.log_num_of_bits),
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_no_overlap_chunked(&spec_1, &spec_1).is_err());
        assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_ok());

        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 1,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_err());

        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: 0,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };
        let spec_2 = SideMetadataSpec {
            scope: SideMetadataScope::Global,
            offset: meta_bytes_per_chunk(spec_1.log_min_obj_size, spec_1.log_num_of_bits) - 1,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_no_overlap_chunked(&spec_1, &spec_2).is_err());
    }

    #[cfg(target_pointer_width = "32")]
    #[test]
    fn test_side_metadata_sanity_verify_local_specs_size() {
        let spec_1 = SideMetadataSpec {
            scope: SideMetadataScope::PolicySpecific,
            offset: 0,
            log_min_obj_size: 0,
            log_num_of_bits: 0,
        };

        assert!(verify_local_specs_size(&[spec_1]).is_ok());
        assert!(verify_local_specs_size(&[spec_1, spec_1]).is_err());
        assert!(verify_local_specs_size(&[spec_1, spec_1, spec_1, spec_1, spec_1]).is_err());
    }
}
